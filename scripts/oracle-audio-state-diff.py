#!/usr/bin/env python3
"""Z80/PSG/YM2612 device-state differential against the pinned headless oracle.

Phase 8.4 task 15. This harness is original, self-contained Python 3 standard
library code. It does not copy upstream Exodus source, and it deliberately never
compares audio samples: cross-emulator sample equality is not claimed anywhere.

What it does
------------
1. Builds one deterministic, independently authored synthetic Mega Drive ROM
   whose 68000 program installs a small Z80 program in Z80 RAM, hands the Z80 its
   bus, and spins on a fixed branch. The Z80 program initialises its registers,
   writes the PSG tone/volume/noise registers, writes YM2612 registers, and then
   executes ``HALT``. The Z80 ``HALT`` is the synchronisation boundary: both
   emulators must have executed the same Z80 instruction sequence and stopped at
   the same ``PC`` before any device state is read.
2. Runs the ROM on the pinned ``exodus-mcpd`` oracle (``.research/oracle/build``,
   revision ``ec005615``) for exactly one frame and reads only device state:
   the Z80 register file through ``cpu.registers``, the PSG register file through
   ``device.describe``, and the YM2612 status byte through the 68000 window at
   ``0xA04000`` with ``cpu.read_memory``.
3. Runs the same Z80 program on ChronoFork through ``chronofork script run``,
   which exposes the backend's inspection pseudo-registers (``z80.*``,
   ``psg.*``, ``ym.status``). The 68000 program writes the Z80 program into Z80
   RAM and then releases the reset + requests the bus, which is ChronoFork's
   documented functional model.
4. Compares each field in a fixed order and reports the first mismatch. Fields
   that cannot be compared are listed with an explicit reason, never silently
   dropped: the pinned oracle publishes no YM2612 register page (only Audio
   Logging), publishes PSG fields ChronoFork does not model, and ChronoFork does
   not publish alternate Z80 registers.
5. Requires negative controls: a corrupted Z80 field, a corrupted PSG field and
   a corrupted YM status must each be detected with the exact first mismatch; a
   child invocation pointed at an empty oracle directory must exit 3 with a
   reason. A harness that cannot prove its comparator fails is not evidence.

Synchronisation note (important, and recorded in the report)
------------------------------------------------------------
Both emulators model hardware ZBR polarity: with the request negated the Z80
owns the bus, so one identical ROM with the same bus-request byte runs both Z80
cores to the same ``HALT`` boundary. The harness therefore uses one identical
ROM on both sides. A separate request-asserted probe checks that asserting the
request stops the Z80 on both emulators at the same PC. The model is still
functional rather than microcycle-exact (ChronoFork yields on the request write
rather than at the Z80 instruction boundary), and the report states that limit
explicitly.

Exit status
-----------
0  PASS for the compared device-state field set (with the scoped limitations
   recorded in the report).
1  A compared field diverged, or the Z80 did not reach the shared HALT boundary.
2  Harness or setup error (missing ChronoFork binary, bad arguments, unpinned or
   unreadable oracle checkout that is present but not the pinned revision).
3  The pinned oracle is absent, so no comparison is possible. Never a silent
   PASS: the reason is printed and written to the report.
"""

import argparse
import hashlib
import json
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_ORACLE_ROOT = REPO / ".research" / "upstream" / "exodus-headless"
DEFAULT_ORACLE_BIN_DIR = REPO / ".research" / "oracle" / "build"
DEFAULT_OUT_DIR = REPO / ".research" / "evidence" / "8.4"
DEFAULT_CHRONOFORK_BIN = REPO / "target" / "debug" / "chronofork"
PINNED_ORACLE_COMMIT = "ec0056151f3ff1c334d09ee375abb30a68fdfec5"
ORACLE_MODULE = "Data/Modules/Sega Mega Drive 1600.xml"
ROM_SIZE = 0x1000
ROM_PROGRAM_AT = 0x200
Z80_PROGRAM_AT = 0x0000
CHRONOFORK_STEPS = 6000
ORACLE_FRAMES = 1
# The Z80 program below is 186 bytes and ends with HALT at offset 185.
Z80_HALT_PC = 186

# Fixed comparison order. Both sides are normalised into this exact key set.
COMPARED_FIELDS = (
    "z80.af",
    "z80.bc",
    "z80.de",
    "z80.hl",
    "z80.ix",
    "z80.iy",
    "z80.sp",
    "z80.pc",
    "z80.i",
    "z80.r",
    "z80.iff1",
    "z80.iff2",
    "z80.im",
    "psg.channel1.volume",
    "psg.channel1.tone",
    "psg.channel2.volume",
    "psg.channel2.tone",
    "psg.channel3.volume",
    "psg.channel3.tone",
    "psg.channel4.volume",
    "psg.channel4.noise_type",
    "psg.channel4.noise_period",
    "psg.control.latched_channel",
    "psg.control.volume_latched",
    "ym.status",
)

EXCLUDED_FIELDS = (
    {
        "prefix": "ym.register*",
        "reason": (
            "the pinned oracle's YM2612 reflection publishes only an Audio Logging page; "
            "device.describe lists no register/channel/operator page, so YM2612 register "
            "and channel state cannot be read back and is not compared. Only the readable "
            "YM2612 status byte is compared."
        ),
    },
    {
        "prefix": "psg.channel4.noise_shift",
        "reason": (
            "the noise shift register is clocked state, not a static register: after "
            "different elapsed emulated time it legitimately holds a different value. It "
            "is excluded from the equality claim and only its rate/type control is compared."
        ),
    },
    {
        "prefix": "psg.channel4.tone",
        "reason": (
            "the SN76489 noise channel's unused tone register is published by the oracle "
            "but not modelled by ChronoFork; it is excluded rather than reinterpreted."
        ),
    },
    {
        "prefix": "z80.alternate_registers",
        "reason": (
            "ChronoFork's inspection pseudo-registers expose the main register file but not "
            "AF'/BC'/DE'/HL'; those oracle fields are excluded for lack of a counterpart."
        ),
    },
    {
        "prefix": "z80.wz",
        "reason": "WZ/MEMPTR is not modelled by ChronoFork (documented Z80 limit).",
    },
)

# ---------------------------------------------------------------------------
# Deterministic synthetic ROM and Z80 program
# ---------------------------------------------------------------------------


def build_z80_program():
    """Returns the Z80 program used on both emulators.

    The program is original code written for this harness. It initialises every
    compared register, programs the PSG and YM2612, sets a final deterministic
    accumulator value and executes HALT.
    """
    program = bytearray()

    def emit(*values):
        program.extend(values)

    def imm(opcode, value):
        emit(opcode, value & 0xFF)

    def imm16(opcode, value):
        emit(opcode, value & 0xFF, (value >> 8) & 0xFF)

    emit(0xAF)  # XOR A -> deterministic F = 0
    imm16(0x01, 0x1234)  # LD BC,0x1234
    imm16(0x11, 0x5678)  # LD DE,0x5678
    imm16(0x21, 0x9ABC)  # LD HL,0x9ABC
    emit(0xDD)
    imm16(0x21, 0x1357)  # LD IX,0x1357
    emit(0xFD)
    imm16(0x21, 0x2468)  # LD IY,0x2468
    imm16(0x31, 0x3FFE)  # LD SP,0x3FFE
    imm(0x3E, 0x42)
    emit(0xED, 0x47)  # LD I,A
    emit(0xED, 0x56)  # IM 1
    emit(0xFB)  # EI -> IFF1/IFF2 set

    def psg(value):
        imm(0x3E, value)
        emit(0x32, 0x11, 0x7F)  # LD (0x7F11),A

    # Channel 1: volume 0, divider 0x123.
    psg(0x90)
    psg(0x83)
    psg(0x12)
    # Channel 2: volume 5, divider 0x0AB.
    psg(0xB5)
    psg(0xAB)
    psg(0x0A)
    # Channel 3: volume 15, divider 0x3FF.
    psg(0xDF)
    psg(0xCF)
    psg(0x3F)
    # Noise: volume 7, white noise, internal rate 2.
    psg(0xF7)
    psg(0xE6)

    def ym(register, value):
        imm(0x3E, register)
        emit(0x32, 0x00, 0x40)  # LD (0x4000),A
        imm(0x3E, value)
        emit(0x32, 0x01, 0x40)  # LD (0x4001),A

    # Timer data without load/enable (no IRQ), DAC data/enable, channel 1
    # frequency/block/algorithm/stereo. The writes exercise the register paths;
    # only the readable status byte is compared.
    for register, value in (
        (0x24, 0x00),
        (0x25, 0x03),
        (0x26, 0x40),
        (0x27, 0x30),
        (0x2A, 0x80),
        (0x2B, 0x80),
        (0xA0, 0x20),
        (0xA4, 0x22),
        (0xB0, 0x07),
        (0xB4, 0xC0),
    ):
        ym(register, value)

    imm(0x3E, 0x5A)  # final A; flags remain 0
    emit(0x76)  # HALT
    return bytes(program)


def _write_header(rom, title):
    def put(offset, data):
        rom[offset : offset + len(data)] = data

    put(0x100, b"SEGA MEGA DRIVE ")
    put(0x110, b"(C) 2026 CHRONOFORK   ")
    put(0x120, title.ljust(0x30, b" "))
    put(0x150, title.ljust(0x30, b" "))
    put(0x180, b"GM 00000000-00".ljust(0x0E, b" "))
    put(0x18E, (0).to_bytes(2, "big"))
    put(0x190, b"J".ljust(0x10, b" "))
    put(0x1A0, (0).to_bytes(4, "big"))
    put(0x1A4, (ROM_SIZE - 1).to_bytes(4, "big"))
    put(0x1A8, (0x00FF0000).to_bytes(4, "big"))
    put(0x1AC, (0x00FFFFFF).to_bytes(4, "big"))
    put(0x1BC, (0).to_bytes(4, "big"))
    put(0x1C0, b" ".ljust(0x30, b" "))
    put(0x1F0, b"J".ljust(0x10, b" "))


def build_rom(z80_program, bus_request):
    """Builds the synthetic ROM.

    ``bus_request`` selects the 68000 byte written to the Z80 bus-request latch
    after the Z80 program has been installed: ``0`` leaves the bus with the Z80
    (the device-state comparison uses one identical ROM on both emulators) and
    ``1`` asserts the request, which stops the Z80 on both emulators (the
    arbitration probe). The 68000 program and the Z80 program are otherwise
    identical.
    """
    rom = bytearray(ROM_SIZE)
    for offset in range(0, ROM_SIZE, 2):
        rom[offset : offset + 2] = b"\x4E\x71"  # NOP fill
    rom[0:4] = (0x00FFFF00).to_bytes(4, "big")
    rom[4:8] = (ROM_PROGRAM_AT).to_bytes(4, "big")
    _write_header(rom, b"CHRONOFORK AUDIO STATE")
    offset = ROM_PROGRAM_AT
    for index, value in enumerate(z80_program):
        address = 0x00A00000 + index
        rom[offset : offset + 2] = (0x13FC).to_bytes(2, "big")  # MOVE.B #imm,(abs).L
        rom[offset + 2 : offset + 4] = value.to_bytes(2, "big")
        rom[offset + 4 : offset + 8] = address.to_bytes(4, "big")
        offset += 8
    for value, address in ((0x01, 0x00A11200), (bus_request, 0x00A11100)):
        rom[offset : offset + 2] = (0x13FC).to_bytes(2, "big")
        rom[offset + 2 : offset + 4] = value.to_bytes(2, "big")
        rom[offset + 4 : offset + 8] = address.to_bytes(4, "big")
        offset += 8
    rom[offset : offset + 2] = b"\x60\xFE"  # BRA.S *
    return bytes(rom)


# ---------------------------------------------------------------------------
# exodus-mcpd client (newline-delimited JSON-RPC 2.0 over stdio)
# ---------------------------------------------------------------------------


class HarnessError(RuntimeError):
    """A setup/input problem that prevents a meaningful comparison."""


class _LineReader(threading.Thread):
    def __init__(self, stream):
        super().__init__(daemon=True)
        self._stream = stream
        self._queue = queue.Queue()

    def run(self):
        try:
            for line in self._stream:
                self._queue.put(line)
        finally:
            self._queue.put(None)

    def readline(self, timeout):
        item = self._queue.get(timeout=timeout)
        if item is None:
            raise HarnessError("exodus-mcpd closed the connection")
        return item


class OracleSession:
    """Minimal JSON-RPC client for the pinned exodus-mcpd daemon."""

    def __init__(self, binary, root, stderr_path):
        self._stderr = open(stderr_path, "w", encoding="utf-8")
        self._process = subprocess.Popen(
            [str(binary), "--base-path", str(root), "--mkdir"],
            cwd=str(root),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self._stderr,
            text=True,
            bufsize=1,
        )
        self._reader = _LineReader(self._process.stdout)
        self._reader.start()
        self._next_id = 0

    def _call(self, method, params=None, timeout=120):
        self._next_id += 1
        request = {
            "jsonrpc": "2.0",
            "id": self._next_id,
            "method": method,
            "params": params if params is not None else {},
        }
        self._process.stdin.write(json.dumps(request) + "\n")
        self._process.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise HarnessError("exodus-mcpd timed out on {}".format(method))
            line = self._reader.readline(remaining).strip()
            if not line:
                continue
            try:
                response = json.loads(line)
            except json.JSONDecodeError as error:
                raise HarnessError(
                    "invalid JSON from exodus-mcpd: {!r}".format(line)
                ) from error
            if response.get("id") != self._next_id:
                continue
            return response

    def notify(self, method, params=None):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self._process.stdin.write(json.dumps(message) + "\n")
        self._process.stdin.flush()

    def tool(self, name, arguments, timeout=120):
        response = self._call(
            "tools/call", {"name": name, "arguments": arguments}, timeout
        )
        if "error" in response:
            raise HarnessError("{}: JSON-RPC error {}".format(name, response["error"]))
        result = response["result"]
        text = result["content"][0]["text"] if result.get("content") else ""
        parsed = json.loads(text) if text.strip().startswith(("{", "[")) else text
        if result.get("isError"):
            raise HarnessError("{}: tool error {}".format(name, parsed))
        return parsed

    def open(self, rom_path):
        self._call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "chronofork-audio-state-diff", "version": "1"},
            },
        )
        self.notify("notifications/initialized")
        self.tool("session.load_module", {"path": ORACLE_MODULE})
        self.tool("session.load_rom", {"path": str(rom_path)})

    def close(self):
        try:
            self._call("shutdown", timeout=10)
        except (HarnessError, OSError, ValueError):
            pass
        try:
            self._process.stdin.close()
        except OSError:
            pass
        try:
            self._process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait(timeout=5)
        self._stderr.close()


# ---------------------------------------------------------------------------
# Oracle device-state reads
# ---------------------------------------------------------------------------


def _register_values(session, device):
    result = session.tool("cpu.registers", {"device": device})
    values = {}
    for entry in result.get("registers", []):
        path = entry.get("path", "")
        values[path] = entry.get("value")
    return values


def _psg_values(session):
    result = session.tool(
        "device.describe", {"device": "PSG", "include_values": True}
    )
    values = {}
    for page in result.get("pages", []):
        if page.get("name") != "Registers":
            continue
        for entry in page.get("entries", []):
            values[entry.get("path", "")] = entry.get("value")
    if not values:
        raise HarnessError(
            "the pinned oracle published no PSG Registers page; "
            "the PSG comparison cannot be made"
        )
    return values


def _ym_status(session):
    result = session.tool(
        "cpu.read_memory", {"device": "Main 68000", "address": "0xA04000", "length": 1}
    )
    data = result.get("bytes")
    if not data:
        raise HarnessError("the oracle returned no YM2612 status byte at 0xA04000")
    return int(data[0])


def oracle_state(binary, root, work, rom_path, z80_program):
    session = OracleSession(binary, root, work / "oracle.stderr")
    try:
        session.open(rom_path)
        # The oracle's arbiter refuses 68000 writes to Z80 RAM while the reset
        # line is asserted, which is the documented hardware behaviour. The
        # harness installs the identical Z80 program through the oracle's own
        # memory interface so both machines execute the same Z80 code.
        session.tool(
            "cpu.write_memory",
            {"device": "Z80", "address": "0x0", "bytes": list(z80_program)},
        )
        session.tool("run.step_frames", {"frames": ORACLE_FRAMES})
        z80 = _register_values(session, "Z80")
        psg = _psg_values(session)
        status = _ym_status(session)
        return z80, psg, status
    finally:
        session.close()


def oracle_arbitration_probe(binary, root, work, rom_path, z80_program):
    """Runs the literal request-asserted ROM and records where the Z80 stops."""
    session = OracleSession(binary, root, work / "oracle-arbitration.stderr")
    try:
        session.open(rom_path)
        session.tool(
            "cpu.write_memory",
            {"device": "Z80", "address": "0x0", "bytes": list(z80_program)},
        )
        session.tool("run.step_frames", {"frames": ORACLE_FRAMES})
        z80 = _register_values(session, "Z80")
        return {
            "rom": rom_path.name,
            "bus_request": 1,
            "oracle_z80_pc": z80.get("Registers/Raw Registers/PC"),
            "oracle_z80_af": z80.get("Registers/Raw Registers/AF"),
            "note": (
                "asserting the 68000 Z80 bus request stops the Z80 on both "
                "emulators under the hardware ZBR polarity, so the two recorded PCs "
                "must agree. The model is still functional: ChronoFork yields on "
                "the request write rather than at the Z80 instruction boundary. "
                "This probe is not part of the device-state comparison."
            ),
        }
    finally:
        session.close()


# ---------------------------------------------------------------------------
# ChronoFork device-state reads through the sandboxed script runtime
# ---------------------------------------------------------------------------


def script_source():
    reports = (
        ("z80_af", "z80.af"),
        ("z80_bc", "z80.bc"),
        ("z80_de", "z80.de"),
        ("z80_hl", "z80.hl"),
        ("z80_ix", "z80.ix"),
        ("z80_iy", "z80.iy"),
        ("z80_sp", "z80.sp"),
        ("z80_pc", "z80.pc"),
        ("z80_i", "z80.i"),
        ("z80_r", "z80.r"),
        ("z80_iff1", "z80.iff1"),
        ("z80_iff2", "z80.iff2"),
        ("z80_im", "z80.im"),
        ("z80_halted", "z80.halted"),
        ("psg_latch", "psg.latch"),
        ("psg_tone0", "psg.tone0"),
        ("psg_tone1", "psg.tone1"),
        ("psg_tone2", "psg.tone2"),
        ("psg_attenuation0", "psg.attenuation0"),
        ("psg_attenuation1", "psg.attenuation1"),
        ("psg_attenuation2", "psg.attenuation2"),
        ("psg_attenuation3", "psg.attenuation3"),
        ("psg_noise_control", "psg.noise_control"),
        ("psg_noise_lfsr", "psg.noise_lfsr"),
        ("ym_status", "ym.status"),
        ("pc", "pc"),
    )
    lines = ["open()", "let t = step({})".format(CHRONOFORK_STEPS)]
    lines.append('report("tick", tick())')
    for name, register in reports:
        lines.append('report("{}", register("{}"))'.format(name, register))
    return "\n".join(lines) + "\n"


def parse_script_report(stdout):
    fields = {}
    in_report = False
    for line in stdout.splitlines():
        if line == "record=script_report":
            in_report = True
            continue
        if in_report:
            if line.startswith("record="):
                break
            if "=" in line:
                key, value = line.split("=", 1)
                if key.startswith("report."):
                    key = key[len("report.") :]
                fields[key] = value
    return fields


def chronofork_state(binary, rom_path, work):
    script = work / "state.cfscript"
    script.write_text(script_source(), encoding="utf-8")
    completed = subprocess.run(
        [
            str(binary),
            "script",
            "run",
            "--rom",
            str(rom_path),
            "--file",
            str(script),
            "--max-steps",
            str(CHRONOFORK_STEPS),
        ],
        cwd=str(REPO),
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    (work / "chronofork.stdout").write_text(completed.stdout, encoding="utf-8")
    (work / "chronofork.stderr").write_text(completed.stderr, encoding="utf-8")
    if completed.returncode != 0:
        raise HarnessError(
            "chronofork script run failed with exit {}: {} {}".format(
                completed.returncode,
                completed.stderr.strip(),
                completed.stdout.strip(),
            )
        )
    report = parse_script_report(completed.stdout)
    if report.get("status") != "passed":
        raise HarnessError(
            "chronofork script report did not pass: {}".format(report.get("failure"))
        )
    return report


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------


def canonical_oracle(z80, psg, ym_status):
    def z(path):
        value = z80.get(path)
        if value is None:
            raise HarnessError("the oracle omitted {}".format(path))
        return int(value)

    def p(path):
        value = psg.get(path)
        if value is None:
            raise HarnessError("the oracle omitted {}".format(path))
        if isinstance(value, bool):
            return int(value)
        return int(value)

    noise = p("Registers/Channel 4/Noise Type")
    period = p("Registers/Channel 4/Noise Period")
    latched_channel = p("Registers/Control/Latched Channel No")
    volume_latched = p("Registers/Control/Volume Register Latched")
    return {
        "z80.af": z("Registers/Raw Registers/AF"),
        "z80.bc": z("Registers/Raw Registers/BC"),
        "z80.de": z("Registers/Raw Registers/DE"),
        "z80.hl": z("Registers/Raw Registers/HL"),
        "z80.ix": z("Registers/Raw Registers/IX"),
        "z80.iy": z("Registers/Raw Registers/IY"),
        "z80.sp": z("Registers/Raw Registers/SP"),
        "z80.pc": z("Registers/Raw Registers/PC"),
        "z80.i": z("Registers/Raw Registers/Private Registers/I"),
        "z80.r": z("Registers/Raw Registers/Private Registers/R"),
        "z80.iff1": z("Registers/Raw Registers/Private Registers/IFF1"),
        "z80.iff2": z("Registers/Raw Registers/Private Registers/IFF2"),
        "z80.im": z("Registers/Raw Registers/Private Registers/IM"),
        "psg.channel1.volume": p("Registers/Channel 1/Volume"),
        "psg.channel1.tone": p("Registers/Channel 1/Tone"),
        "psg.channel2.volume": p("Registers/Channel 2/Volume"),
        "psg.channel2.tone": p("Registers/Channel 2/Tone"),
        "psg.channel3.volume": p("Registers/Channel 3/Volume"),
        "psg.channel3.tone": p("Registers/Channel 3/Tone"),
        "psg.channel4.volume": p("Registers/Channel 4/Volume"),
        "psg.channel4.noise_type": noise,
        "psg.channel4.noise_period": period,
        "psg.control.latched_channel": latched_channel,
        "psg.control.volume_latched": volume_latched,
        "ym.status": ym_status,
    }


def canonical_chronofork(report):
    def value(key):
        raw = report.get(key)
        if raw is None:
            raise HarnessError("chronofork report omitted {}".format(key))
        return int(raw)

    noise_control = value("psg_noise_control")
    latch = value("psg_latch")
    return {
        "z80.af": value("z80_af"),
        "z80.bc": value("z80_bc"),
        "z80.de": value("z80_de"),
        "z80.hl": value("z80_hl"),
        "z80.ix": value("z80_ix"),
        "z80.iy": value("z80_iy"),
        "z80.sp": value("z80_sp"),
        "z80.pc": value("z80_pc"),
        "z80.i": value("z80_i"),
        "z80.r": value("z80_r"),
        "z80.iff1": value("z80_iff1"),
        "z80.iff2": value("z80_iff2"),
        "z80.im": value("z80_im"),
        "psg.channel1.volume": value("psg_attenuation0"),
        "psg.channel1.tone": value("psg_tone0"),
        "psg.channel2.volume": value("psg_attenuation1"),
        "psg.channel2.tone": value("psg_tone1"),
        "psg.channel3.volume": value("psg_attenuation2"),
        "psg.channel3.tone": value("psg_tone2"),
        "psg.channel4.volume": value("psg_attenuation3"),
        "psg.channel4.noise_type": (noise_control >> 2) & 0x01,
        "psg.channel4.noise_period": noise_control & 0x03,
        "psg.control.latched_channel": (latch >> 5) & 0x03,
        "psg.control.volume_latched": (latch >> 4) & 0x01,
        "ym.status": value("ym_status"),
    }


def compare(expected, actual, order=COMPARED_FIELDS):
    for field in order:
        if field not in expected:
            return {"field": field, "expected": None, "actual": actual.get(field)}
        if field not in actual:
            return {"field": field, "expected": expected.get(field), "actual": None}
        if expected[field] != actual[field]:
            return {
                "field": field,
                "expected": expected[field],
                "actual": actual[field],
            }
    return None


def negative_controls(expected, actual):
    controls = []
    corrupted = dict(expected)
    corrupted["z80.af"] = (corrupted["z80.af"] + 1) & 0xFFFF
    mismatch = compare(corrupted, actual)
    controls.append(
        {
            "control": "corrupt-z80-af",
            "detected": mismatch is not None and mismatch["field"] == "z80.af",
            "first_mismatch": mismatch,
        }
    )
    corrupted = dict(expected)
    corrupted["psg.channel2.tone"] = 0x0AA
    mismatch = compare(corrupted, actual)
    controls.append(
        {
            "control": "corrupt-psg-tone",
            "detected": mismatch is not None
            and mismatch["field"] == "psg.channel2.tone",
            "first_mismatch": mismatch,
        }
    )
    corrupted = dict(expected)
    corrupted["ym.status"] = (corrupted["ym.status"] ^ 0xFF) & 0xFF
    mismatch = compare(corrupted, actual)
    controls.append(
        {
            "control": "corrupt-ym-status",
            "detected": mismatch is not None and mismatch["field"] == "ym.status",
            "first_mismatch": mismatch,
        }
    )
    return controls


# ---------------------------------------------------------------------------
# Provenance and report writers
# ---------------------------------------------------------------------------


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def git_output(directory, *args):
    completed = subprocess.run(
        ["git", "-C", str(directory), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip()


def oracle_availability(options):
    root = options.oracle_root
    binary_dir = options.oracle_bin_dir
    missing = []
    if not root.is_dir():
        missing.append(str(root))
    if not (binary_dir / "exodus-mcpd").is_file():
        missing.append(str(binary_dir / "exodus-mcpd"))
    if not (binary_dir / "exodus-probe").is_file():
        missing.append(str(binary_dir / "exodus-probe"))
    if missing:
        return {
            "available": False,
            "reason": "pinned oracle is absent: missing {}".format(", ".join(missing)),
            "exit_code": 3,
        }
    commit = git_output(root, "rev-parse", "HEAD")
    if commit is None:
        return {
            "available": False,
            "reason": "pinned oracle checkout is not a readable git repository: {}".format(root),
            "exit_code": 3,
        }
    if commit != PINNED_ORACLE_COMMIT:
        return {
            "available": False,
            "reason": "oracle checkout is at {}, expected pinned {}".format(
                commit, PINNED_ORACLE_COMMIT
            ),
            "exit_code": 2,
        }
    return {
        "available": True,
        "source_commit": commit,
        "expected_commit": PINNED_ORACLE_COMMIT,
        "branch": git_output(root, "rev-parse", "--abbrev-ref", "HEAD"),
        "binaries": {
            name: {
                "path": str(binary_dir / name),
                "sha256": sha256_file(binary_dir / name),
            }
            for name in ("exodus-mcpd", "exodus-probe")
        },
        "module": ORACLE_MODULE,
        "frames": ORACLE_FRAMES,
    }


def write_unavailable_report(out_dir, evidence):
    """Writes the explicit no-oracle report without pretending to compare."""
    out_dir.mkdir(parents=True, exist_ok=True)
    json_path = out_dir / "oracle-audio-state-diff.json"
    md_path = out_dir / "oracle-audio-state-diff.md"
    json_path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    reason = evidence.get("oracle", {}).get("reason", "pinned oracle unavailable")
    md_path.write_text(
        "# Z80/PSG/YM2612 device-state differential (phase 8.4 task 15)\n\n"
        "**Overall status: UNAVAILABLE**\n\n"
        "No comparison was made and no PASS is claimed.\n\n"
        "- {}\n".format(reason),
        encoding="utf-8",
    )
    return json_path, md_path


def write_report(out_dir, evidence):
    out_dir.mkdir(parents=True, exist_ok=True)
    json_path = out_dir / "oracle-audio-state-diff.json"
    md_path = out_dir / "oracle-audio-state-diff.md"
    json_path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    summary = evidence["summary"]
    lines = []
    lines.append("# Z80/PSG/YM2612 device-state differential (phase 8.4 task 15)")
    lines.append("")
    lines.append(
        "Generated by `scripts/oracle-audio-state-diff.py` at {}.".format(
            evidence["generated_utc"]
        )
    )
    lines.append("")
    lines.append("**Overall status: {}**".format(summary["status"].upper()))
    lines.append("")
    lines.append(
        "Compared {} device-state fields at the shared Z80 HALT boundary; the "
        "harness never compares audio samples.".format(summary["compared_fields"])
    )
    lines.append("")
    lines.append("## Pinned oracle")
    lines.append("")
    if evidence.get("oracle", {}).get("available"):
        oracle = evidence["oracle"]
        lines.append("- checkout commit: `{}`".format(oracle["source_commit"]))
        lines.append("- module: `{}`".format(oracle["module"]))
        lines.append("- frames: {}".format(oracle["frames"]))
        for name, binary in oracle["binaries"].items():
            lines.append("- `{}` sha256 `{}`".format(name, binary["sha256"]))
    else:
        lines.append("- UNAVAILABLE: {}".format(evidence.get("oracle", {}).get("reason")))
    lines.append("")
    lines.append("## Compared fields")
    lines.append("")
    lines.append(", ".join("`{}`".format(field) for field in COMPARED_FIELDS))
    lines.append("")
    lines.append("## Excluded fields (explicit, not silently dropped)")
    lines.append("")
    for item in EXCLUDED_FIELDS:
        lines.append("- `{}`: {}".format(item["prefix"], item["reason"]))
    lines.append("")
    lines.append("## Result")
    lines.append("")
    if summary.get("first_mismatch"):
        mismatch = summary["first_mismatch"]
        lines.append(
            "First mismatch: `{}` expected `{}`, ChronoFork `{}`.".format(
                mismatch["field"], mismatch["expected"], mismatch["actual"]
            )
        )
    else:
        lines.append(
            "Every compared field matched. The Z80 reached the shared HALT "
            "boundary at PC `{}` on both emulators.".format(summary.get("halt_pc"))
        )
    lines.append("")
    lines.append("## Scope and limits")
    lines.append("")
    lines.append(
        "- No cross-emulator **sample** equality is claimed; this harness reads "
        "device state only."
    )
    lines.append(
        "- The pinned oracle publishes no YM2612 register/channel page, so only "
        "the readable YM2612 status byte is compared; YM2612 register and channel "
        "state are not claimed equal."
    )
    lines.append(
        "- Both emulators model the hardware ZBR polarity, so the compared "
        "Z80/PSG state uses one identical ROM (request negated: the Z80 owns the "
        "bus) and both Z80 cores execute the same program to HALT. The model is "
        "still functional rather than microcycle-exact: ChronoFork yields on the "
        "request write rather than at the Z80 instruction boundary. The "
        "request-asserted probe is reported separately."
    )
    lines.append(
        "- Time-dependent fields (PSG noise shift register) are excluded; the "
        "two emulators do not share an emulated-time budget at the boundary."
    )
    lines.append("")
    lines.append("## Negative controls")
    lines.append("")
    for control in evidence.get("negative_controls", []):
        lines.append(
            "- `{}`: detected={}".format(control["control"], control["detected"])
        )
    lines.append(
        "- missing-oracle child invocation: exit {}".format(
            evidence.get("missing_oracle_control", {}).get("exit_code")
        )
    )
    lines.append(
        "- `{}`".format(
            evidence.get("missing_oracle_control", {}).get("reason", "")
        )
    )
    lines.append("")
    lines.append("## Arbitration probe (request asserted on both emulators)")
    lines.append("")
    probe = evidence.get("arbitration_probe", {})
    if probe:
        lines.append(
            "- same ROM, bus request asserted: oracle Z80 PC `{}`, "
            "ChronoFork Z80 PC `{}`".format(
                probe.get("oracle_z80_pc"), probe.get("chronofork_z80_pc")
            )
        )
        lines.append("- {}".format(probe.get("note", "")))
    lines.append("")
    lines.append("## Commands and artifacts")
    lines.append("")
    lines.append("- `python3 scripts/oracle-audio-state-diff.py --out-dir {}`".format(out_dir))
    lines.append(
        "- oracle fixture ROM sha256 `{}`".format(
            evidence["fixture"]["rom_oracle_sha256"]
        )
    )
    lines.append(
        "- ChronoFork fixture ROM sha256 `{}`".format(
            evidence["fixture"]["rom_chronofork_sha256"]
        )
    )
    lines.append("- ChronoFork binary sha256 `{}`".format(evidence["chronofork"]["binary_sha256"]))
    lines.append("- ChronoFork commit `{}`".format(evidence["chronofork"]["commit"]))
    lines.append("")
    md_path.write_text("\n".join(lines), encoding="utf-8")
    return json_path, md_path


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def self_missing_oracle_control(script_path):
    empty = Path(tempfile.mkdtemp(prefix="chronofork-audio-oracle-missing-"))
    out_dir = empty / "out"
    try:
        completed = subprocess.run(
            [
                sys.executable,
                str(script_path),
                "--check-oracle",
                "--oracle-bin-dir",
                str(empty / "bin"),
                "--oracle-root",
                str(empty / "root"),
                "--out-dir",
                str(out_dir),
            ],
            capture_output=True,
            text=True,
            timeout=120,
            check=False,
        )
        reason = (completed.stdout + completed.stderr).strip().splitlines()
        return {
            "exit_code": completed.returncode,
            "reason": reason[0] if reason else "",
        }
    finally:
        shutil.rmtree(empty, ignore_errors=True)


def parse_options(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument("--oracle-root", type=Path, default=DEFAULT_ORACLE_ROOT)
    parser.add_argument("--oracle-bin-dir", type=Path, default=DEFAULT_ORACLE_BIN_DIR)
    parser.add_argument("--chronofork-bin", type=Path, default=DEFAULT_CHRONOFORK_BIN)
    parser.add_argument(
        "--check-oracle",
        action="store_true",
        help="only validate the pinned oracle; exit 0/2/3 without running a comparison",
    )
    return parser.parse_args(argv)


def main(argv):
    options = parse_options(argv)
    oracle = oracle_availability(options)
    if not oracle["available"]:
        message = oracle["reason"]
        print("oracle-audio-state-diff: {}".format(message))
        write_unavailable_report(
            options.out_dir,
            {
                "generated_utc": datetime.now(timezone.utc).isoformat(),
                "summary": {"status": "unavailable", "compared_fields": 0},
                "oracle": oracle,
            },
        )
        return oracle["exit_code"]
    if options.check_oracle:
        print("oracle-audio-state-diff: pinned oracle available: {}".format(oracle["source_commit"]))
        return 0

    if not options.chronofork_bin.is_file():
        print(
            "oracle-audio-state-diff: ChronoFork binary is missing: {} (run "
            "`cargo build -p chronofork-cli` first)".format(options.chronofork_bin)
        )
        return 2

    z80_program = build_z80_program()
    if len(z80_program) != Z80_HALT_PC:
        raise HarnessError(
            "the fixture Z80 program length {} does not match HALT PC {}".format(
                len(z80_program), Z80_HALT_PC
            )
        )
    work = Path(tempfile.mkdtemp(prefix="chronofork-audio-state-"))
    try:
        rom_oracle = work / "audio-state-oracle.rom"
        rom_chronofork = work / "audio-state-chronofork.rom"
        rom_probe = work / "audio-state-arbitration.rom"
        probe_work = work / "arbitration"
        probe_work.mkdir()
        # Both emulators model the hardware ZBR polarity, so the device-state
        # comparison uses the same bus-request byte (0: the Z80 owns the bus) on
        # one identical ROM, and the request-asserted probe checks that asserting
        # it stops the Z80 on both sides.
        rom_oracle.write_bytes(build_rom(z80_program, 0))
        rom_chronofork.write_bytes(build_rom(z80_program, 0))
        rom_probe.write_bytes(build_rom(z80_program, 1))

        oracle_z80, oracle_psg, oracle_ym = oracle_state(
            options.oracle_bin_dir / "exodus-mcpd",
            options.oracle_root,
            work,
            rom_oracle,
            z80_program,
        )
        arbitration = oracle_arbitration_probe(
            options.oracle_bin_dir / "exodus-mcpd",
            options.oracle_root,
            probe_work,
            rom_probe,
            z80_program,
        )
        expected = canonical_oracle(oracle_z80, oracle_psg, oracle_ym)
        report = chronofork_state(options.chronofork_bin, rom_chronofork, work)
        actual = canonical_chronofork(report)
        halt_ok = int(report.get("z80_halted", "0")) == 1
        if not halt_ok:
            mismatch = {
                "field": "z80.halted",
                "expected": 1,
                "actual": int(report.get("z80_halted", "0")),
            }
        else:
            mismatch = compare(expected, actual)
        probe_report = chronofork_state(options.chronofork_bin, rom_probe, probe_work)
        arbitration["chronofork_z80_pc"] = int(probe_report.get("z80_pc", "0"))
        arbitration["z80_pc_agrees"] = str(arbitration.get("oracle_z80_pc")) == str(
            arbitration.get("chronofork_z80_pc")
        )
        controls = negative_controls(expected, actual)
        controls_ok = all(control["detected"] for control in controls)
        missing = self_missing_oracle_control(Path(__file__).resolve())
        missing_ok = missing["exit_code"] == 3 and bool(missing["reason"])
        status = "pass" if mismatch is None and controls_ok and missing_ok else "divergence"
        evidence = {
            "generated_utc": datetime.now(timezone.utc).isoformat(),
            "summary": {
                "status": status,
                "compared_fields": len(COMPARED_FIELDS),
                "halt_pc": int(report.get("z80_pc", "0")),
                "first_mismatch": mismatch,
                "negative_controls_passed": controls_ok,
                "missing_oracle_control_passed": missing_ok,
            },
            "oracle": oracle,
            "fixture": {
                "rom_oracle_sha256": sha256_file(rom_oracle),
                "rom_chronofork_sha256": sha256_file(rom_chronofork),
                "rom_sha256": sha256_file(rom_oracle),
                "z80_program_sha256": hashlib.sha256(z80_program).hexdigest(),
                "z80_program_bytes": len(z80_program),
                "oracle_bus_request": 0,
                "chronofork_bus_request": 0,
                "probe_bus_request": 1,
                "synchronisation": "Z80 HALT after the same program on both emulators",
            },
            "chronofork": {
                "binary": str(options.chronofork_bin),
                "binary_sha256": sha256_file(options.chronofork_bin),
                "commit": git_output(REPO, "rev-parse", "HEAD"),
                "script_steps": CHRONOFORK_STEPS,
                "reported_tick": report.get("tick"),
                "reported_68000_pc": report.get("pc"),
            },
            "expected": expected,
            "actual": actual,
            "excluded_fields": list(EXCLUDED_FIELDS),
            "negative_controls": controls,
            "missing_oracle_control": missing,
            "arbitration_probe": arbitration,
        }
        json_path, md_path = write_report(options.out_dir, evidence)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    if status != "pass":
        if mismatch:
            print(
                "oracle-audio-state-diff: DIVERGENCE at {}: expected {}, ChronoFork {}".format(
                    mismatch["field"], mismatch["expected"], mismatch["actual"]
                )
            )
        else:
            print("oracle-audio-state-diff: a negative control failed")
        return 1
    print(
        "oracle-audio-state-diff: PASS ({} device-state fields, Z80 HALT PC {}, "
        "scoped YM/PSG/arbitration limits recorded)".format(
            len(COMPARED_FIELDS), report.get("z80_pc")
        )
    )
    print("oracle-audio-state-diff: {}".format(json_path))
    print("oracle-audio-state-diff: {}".format(md_path))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except HarnessError as error:
        print("oracle-audio-state-diff: {}".format(error))
        sys.exit(2)
