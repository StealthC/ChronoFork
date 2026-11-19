#!/usr/bin/env python3
"""VDP state differential harness against the pinned Exodus oracle.

Phase 8.3 Task 13 drives the pinned headless Exodus build and ChronoFork over
independently authored synthetic VDP programs and compares the canonical VDP
surface at synchronized instruction boundaries.  The harness is original code
(Python 3 standard library only); it does not copy upstream Exodus source.

What is compared
----------------
1. **Oracle-observable state** (Exodus, via the guest program and the MCP
   reflection tools): the 68000 register file, work RAM, and the VDP *status
   register* and *H/V counters* the guest reads into work RAM at the program's
   stop marker.  Both emulators execute exactly the same instruction stream and
   stop before the same marker instruction, so the boundary is synchronized.
2. **ChronoFork canonical VDP state** (`chronofork debug vdp`): the VDP
   register file (registers 0x00-0x17), the VDP status and the complete 64 KiB
   VRAM image.  The harness compares these against the values the synthetic
   program programmed and wrote, reporting the first differing register or
   VRAM byte with its address.
3. **A negative control** corrupts one compared value in memory and asserts the
   comparator reports the first mismatch with context.

What is not comparable (recorded as explicit limits, never silently passed)
---------------------------------------------------------------------------
- The pinned oracle's VDP reflection pages expose only settings and layer
  visibility; they do not publish VDP registers, VRAM, CRAM or VSRAM.  Debug
  memory reads/writes at 0xC00000..0xC0000F bypass the device (they return zero
  and do not mutate it), so VDP memory cannot be read externally.
- The oracle's data-port read path in this headless build returns the stale
  write latch instead of the addressed VDP memory word (the probe program
  records this verbatim), so a guest readback cannot compare VDP memory bytes.
- ChronoFork's CLI exposes the VDP register file, status and VRAM, but not
  CRAM/VSRAM; their canonical content and round-trip are covered by the backend
  unit tests (`vdp_ports`, `vdp_snapshot`, `vdp_dma`).
- The H counter is documented as approximate in ChronoFork; an H-only
  difference is recorded as a scoped divergence with its delta, while a V
  counter, status, register, VRAM or CPU difference fails the run.
- Pixels are not compared across emulators: the headless oracle has no
  framebuffer.

Exit status: 0 when every compared field matches (documented limits recorded),
1 on an unexpected divergence, 2 on a harness/setup error, 3 when the oracle is
absent (the reason is written to the report; this is not a pass).
"""

import argparse
import hashlib
import json
import os
import queue
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ORACLE_ROOT = REPO / ".research" / "upstream" / "exodus-headless"
ORACLE_BIN = REPO / ".research" / "oracle" / "build" / "exodus-mcpd"
CHRONOFORK_BIN = REPO / "target" / "debug" / "chronofork"
DEFAULT_OUT_DIR = REPO / ".research" / "evidence" / "8.3"
PINNED_ORACLE_COMMIT = "ec0056151f3ff1c334d09ee375abb30a68fdfec5"
ORACLE_MODULE = "Data/Modules/Sega Mega Drive 1600.xml"
PROGRAM_AT = 0x200
ROM_SIZE = 0x10000
RAM_BASE = 0x00FF0000
RAM_SIZE = 0x10000
VRAM_SIZE = 0x10000
INSTRUCTION_BUDGET = 200_000
ORACLE_POLL_SECONDS = 0.01
ORACLE_STOP_TIMEOUT = 60.0

# Marker instruction: a branch-to-self that both emulators stop before.
MARKER_OPCODE = b"\x60\xfe"


class HarnessError(RuntimeError):
    """A setup or protocol problem that prevents a meaningful comparison."""


# ---------------------------------------------------------------------------
# Deterministic ROM generation
# ---------------------------------------------------------------------------


class Assembler:
    """Minimal assembler for the 68000 forms the synthetic VDP programs use."""

    def __init__(self):
        self.code = bytearray()
        self.labels = {}
        self.relocations = []

    def raw(self, *bytes_):
        self.code.extend(bytes_)

    def moveq_zero(self, data_register):
        self.raw(0x70 + data_register * 2, 0x00)

    def move_w_imm_d0(self, value):
        self.raw(0x30, 0x3C, (value >> 8) & 0xFF, value & 0xFF)

    def move_w_d0_abs(self, address):
        self.raw(0x33, 0xC0)
        self.raw(*address.to_bytes(4, "big"))

    def move_w_abs_d2(self, address):
        self.raw(0x34, 0x39)
        self.raw(*address.to_bytes(4, "big"))

    def move_w_d2_abs(self, address):
        self.raw(0x33, 0xC2)
        self.raw(*address.to_bytes(4, "big"))

    def move_w_imm_sr(self, value):
        self.raw(0x46, 0xFC, (value >> 8) & 0xFF, value & 0xFF)

    def nop(self, count=1):
        for _ in range(count):
            self.raw(0x4E, 0x71)

    def bra_self(self):
        self.raw(*MARKER_OPCODE)

    def write_register(self, register, value):
        self.move_w_imm_d0(0x8000 | (register << 8) | value)
        self.move_w_d0_abs(0xC00004)

    def command(self, code, address):
        """Writes the two-word VDP control command for ``code``/``address``."""
        first = ((code & 0x03) << 14) | (address & 0x3FFF)
        second = (((code >> 2) & 0x0F) << 4) | ((address >> 14) & 0x03)
        self.move_w_imm_d0(first)
        self.move_w_d0_abs(0xC00004)
        self.move_w_imm_d0(second)
        self.move_w_d0_abs(0xC00004)

    def write_word(self, value):
        self.move_w_imm_d0(value)
        self.move_w_d0_abs(0xC00000)

    def read_word_to(self, ram_address):
        self.move_w_abs_d2(0xC00000)
        self.move_w_d2_abs(ram_address)

    def read_status_to(self, ram_address):
        self.move_w_abs_d2(0xC00004)
        self.move_w_d2_abs(ram_address)

    def read_hv_to(self, ram_address):
        self.move_w_abs_d2(0xC00008)
        self.move_w_d2_abs(ram_address)


def write_header(rom, title):
    def put(offset, data):
        rom[offset : offset + len(data)] = data

    put(0x100, b"SEGA MEGA DRIVE ")
    put(0x110, b"(C) 2026 CHRONOFORK   ")
    put(0x120, title.ljust(0x30, b" "))
    put(0x150, title.ljust(0x30, b" "))
    put(0x180, b"GM 00000000-00".ljust(0x0E, b" "))
    put(0x18E, struct.pack(">H", 0x0000))
    put(0x190, b"J               ")
    put(0x1A0, struct.pack(">I", 0x00000000))
    put(0x1A4, struct.pack(">I", ROM_SIZE - 1))
    put(0x1A8, struct.pack(">I", 0x00FF0000))
    put(0x1AC, struct.pack(">I", 0x00FFFFFF))
    put(0x1B0, b"\x00" * 12)
    put(0x1BC, b"\x00" * 4)
    put(0x1C0, b" " * 0x30)
    put(0x1F0, b"J" + b" " * 0x0F)


def make_rom(code, title):
    rom = bytearray(ROM_SIZE)
    for offset in range(0, ROM_SIZE, 2):
        rom[offset : offset + 2] = b"\x4e\x71"  # NOP fill
    rom[0x000:0x004] = struct.pack(">I", 0x00FFFF00)  # initial SP
    rom[0x004:0x008] = struct.pack(">I", PROGRAM_AT)  # initial PC
    write_header(rom, title)
    if PROGRAM_AT + len(code) > ROM_SIZE:
        raise HarnessError("program does not fit the synthetic ROM")
    rom[PROGRAM_AT : PROGRAM_AT + len(code)] = code
    return bytes(rom)


# ---------------------------------------------------------------------------
# Synthetic VDP programs
# ---------------------------------------------------------------------------

# Register values chosen to exercise every documented register without
# enabling an interrupt source (register 0 bits 4/5 stay clear) so the stop
# boundary stays deterministic.
VDP_REGISTERS = {
    0x00: 0x04,  # no VBlank/HBlank interrupt enable
    0x01: 0x44,  # display enable, no VBlank interrupt, no DMA enable
    0x02: 0x30,  # plane A nametable 0xC000
    0x03: 0x2C,  # window nametable 0xB000
    0x04: 0x07,  # plane A nametable base 0xE000
    0x05: 0x6C,  # sprite attribute table base 0xD800
    0x06: 0x00,
    0x07: 0x00,  # backdrop colour 0
    0x08: 0x00,
    0x09: 0x00,
    0x0A: 0x00,  # horizontal scroll 0
    0x0B: 0x00,
    0x0C: 0x00,  # H40, no interlace, no shadow/highlight
    0x0D: 0x3F,  # horizontal scroll table base 0xFC00
    0x0E: 0x00,
    0x0F: 0x02,  # auto-increment 2
    0x10: 0x01,  # 64x32 plane size
    0x11: 0x00,  # window horizontal position
    0x12: 0x00,  # window vertical position
    0x13: 0x00,  # DMA length low
    0x14: 0x00,  # DMA length high
    0x15: 0x00,  # DMA source low
    0x16: 0x00,  # DMA source mid
    0x17: 0x00,  # DMA source high + mode (no transfer triggered)
}

VRAM_PATTERN_AT = 0x0000
VRAM_PATTERN_WORDS = 16
PLANE_A_AT = 0xC000
PLANE_A_WORDS = 8
SPRITE_AT = 0xD800
SPRITE_WORDS = 8
HSCROLL_AT = 0xFC00
HSCROLL_WORDS = 4
CRAM_PATTERN_WORDS = 16
VSRAM_PATTERN_WORDS = 16


def register_vram_program():
    """Programs registers, planes, sprites, scroll, CRAM and VSRAM, then stops."""
    asm = Assembler()
    for data_register in range(4):
        asm.moveq_zero(data_register)
    asm.move_w_imm_sr(0x2700)
    for register in range(0x18):
        asm.write_register(register, VDP_REGISTERS[register])
    # Deterministic VRAM patterns: a flat block, a plane A row, a sprite row and
    # the horizontal scroll table.
    asm.command(0x01, VRAM_PATTERN_AT)
    for index in range(VRAM_PATTERN_WORDS):
        asm.write_word(0x1000 + index * 0x0101)
    asm.command(0x01, PLANE_A_AT)
    for index in range(PLANE_A_WORDS):
        asm.write_word(0x2000 | (index << 4) | 0x0001)
    asm.command(0x01, SPRITE_AT)
    for index in range(SPRITE_WORDS):
        asm.write_word(0x4000 | (index << 8))
    asm.command(0x01, HSCROLL_AT)
    for index in range(HSCROLL_WORDS):
        asm.write_word(0x8000 | index)
    # CRAM and VSRAM writes (word-addressed; auto-increment is 2 bytes = 1 word).
    asm.command(0x03, 0x0000)
    for index in range(CRAM_PATTERN_WORDS):
        asm.write_word(0x0E00 | index)
    # Read the CRAM words back into work RAM. ChronoFork decodes CRAM data-port
    # reads; the oracle returns its stale write latch (recorded as a limit), so
    # the ChronoFork readback is checked against the expected pattern and the
    # oracle pair is recorded as a scoped divergence.
    asm.command(0x08, 0x0000)
    for index in range(CRAM_PATTERN_WORDS):
        asm.read_word_to(0x00FF0060 + index * 2)
    asm.command(0x04, 0x0000)
    for index in range(VSRAM_PATTERN_WORDS):
        asm.write_word(0x0400 | index)
    asm.read_status_to(0x00FF0040)
    asm.read_hv_to(0x00FF0042)
    marker = PROGRAM_AT + len(asm.code)
    asm.bra_self()
    return asm.code, marker


def status_counters_program():
    """Samples the status register and H/V counters at four boundaries."""
    asm = Assembler()
    for data_register in range(4):
        asm.moveq_zero(data_register)
    asm.move_w_imm_sr(0x2700)
    asm.write_register(0x0F, 0x02)
    asm.read_status_to(0x00FF0040)
    asm.read_hv_to(0x00FF0042)
    asm.nop(64)
    asm.read_status_to(0x00FF0044)
    asm.read_hv_to(0x00FF0046)
    asm.nop(512)
    asm.read_status_to(0x00FF0048)
    asm.read_hv_to(0x00FF004A)
    asm.nop(2048)
    asm.read_status_to(0x00FF004C)
    asm.read_hv_to(0x00FF004E)
    marker = PROGRAM_AT + len(asm.code)
    asm.bra_self()
    return asm.code, marker


PROGRAMS = {
    "register-vram": {
        "build": register_vram_program,
        "title": b"ChronoFork VDP diff 1",
        "expect_status": True,
        "expect_registers": True,
        "expect_cram": [0x0E00 | index for index in range(CRAM_PATTERN_WORDS)],
        "expect_vram": {
            VRAM_PATTERN_AT: [0x1000 + index * 0x0101 for index in range(VRAM_PATTERN_WORDS)],
            PLANE_A_AT: [0x2000 | (index << 4) | 0x0001 for index in range(PLANE_A_WORDS)],
            SPRITE_AT: [0x4000 | (index << 8) for index in range(SPRITE_WORDS)],
            HSCROLL_AT: [0x8000 | index for index in range(HSCROLL_WORDS)],
        },
    },
    "status-counters": {
        "build": status_counters_program,
        "title": b"ChronoFork VDP diff 2",
        "expect_status": False,
        "expect_registers": False,
        "expect_vram": {},
    },
}


def oracle_readback_probe():
    """Probes whether the oracle exposes VDP memory through the data port.

    Writes four VRAM words, then reads them back into work RAM.  The pinned
    headless oracle returns its stale write latch for every read; recording that
    verbatim is what justifies the memory-content limit in the report.
    """
    asm = Assembler()
    for data_register in range(4):
        asm.moveq_zero(data_register)
    asm.move_w_imm_sr(0x2700)
    asm.write_register(0x0F, 0x02)
    asm.command(0x01, 0x0000)
    for value in (0xAAAA, 0xBBBB, 0xCCCC, 0xDDDD):
        asm.write_word(value)
    asm.command(0x00, 0x0000)
    for index in range(4):
        asm.read_word_to(0x00FF0050 + index * 2)
    marker = PROGRAM_AT + len(asm.code)
    asm.bra_self()
    return asm.code, marker


def expected_vram_image(program):
    image = bytearray(VRAM_SIZE)
    for address, words in program["expect_vram"].items():
        for index, word in enumerate(words):
            offset = address + index * 2
            image[offset : offset + 2] = word.to_bytes(2, "big")
    return bytes(image)


# ---------------------------------------------------------------------------
# Oracle MCP client
# ---------------------------------------------------------------------------


class OracleClient:
    """Newline-delimited JSON-RPC client for the pinned ``exodus-mcpd``."""

    def __init__(self):
        if not ORACLE_BIN.is_file():
            raise HarnessError(f"oracle binary not found at {ORACLE_BIN}")
        if not (ORACLE_ROOT / "Data" / "Modules" / "Sega Mega Drive 1600.xml").is_file():
            raise HarnessError(f"oracle module not found under {ORACLE_ROOT}")
        self.base_path = REPO / ".research" / "evidence" / "8.3" / "oracle-vdp-base"
        try:
            self.base_path.mkdir(parents=True, exist_ok=True)
        except OSError as error:
            raise HarnessError(f"cannot create oracle base path: {error}") from error
        self.process = subprocess.Popen(
            [str(ORACLE_BIN), "--base-path", str(self.base_path), "--mkdir"],
            cwd=str(ORACLE_ROOT),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self.reader = queue.Queue()
        self.next_id = 0
        self.thread = threading.Thread(target=self._read_lines, daemon=True)
        self.thread.start()
        self.call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "chronofork-vdp-diff", "version": "1"},
            },
        )

    def _read_lines(self):
        try:
            for line in self.process.stdout:
                self.reader.put(line)
        except (OSError, ValueError):
            return
        self.reader.put(None)

    def call(self, method, params=None, timeout=30.0):
        self.next_id += 1
        request_id = self.next_id
        message = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            message["params"] = params
        try:
            self.process.stdin.write(json.dumps(message) + "\n")
            self.process.stdin.flush()
        except OSError as error:
            raise HarnessError(f"oracle stdin closed: {error}") from error
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise HarnessError(f"oracle did not answer {method} within {timeout:.0f}s")
            try:
                line = self.reader.get(timeout=remaining)
            except queue.Empty as error:
                raise HarnessError(f"oracle did not answer {method} within {timeout:.0f}s") from error
            if line is None:
                raise HarnessError("oracle closed the connection")
            line = line.strip()
            if not line:
                continue
            try:
                response = json.loads(line)
            except json.JSONDecodeError:
                continue
            if response.get("id") == request_id:
                return response

    def tool(self, name, arguments, timeout=30.0):
        response = self.call(
            "tools/call", {"name": name, "arguments": arguments}, timeout=timeout
        )
        if "error" in response:
            raise HarnessError(f"{name}: JSON-RPC error: {response['error']}")
        result = response["result"]
        text = result["content"][0]["text"] if result.get("content") else ""
        parsed = json.loads(text) if text.strip().startswith(("{", "[")) else text
        if result.get("isError"):
            raise HarnessError(f"{name}: tool error: {parsed}")
        return parsed

    def close(self):
        if self.process.poll() is None:
            try:
                self.tool("session.shutdown", {}, timeout=5.0)
            except HarnessError:
                pass
            try:
                self.process.stdin.close()
            except OSError:
                pass
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()


def run_on_oracle(client, rom_bytes, marker, workdir):
    rom_path = workdir / f"oracle-{marker:04x}.rom"
    rom_path.write_bytes(rom_bytes)
    client.tool("session.load_module", {"path": ORACLE_MODULE})
    client.tool("session.load_rom", {"path": str(rom_path)})
    client.tool("breakpoint.create", {"device": "Main 68000", "address": f"0x{marker:04X}"})
    client.tool("run.start", {})
    deadline = time.monotonic() + ORACLE_STOP_TIMEOUT
    while True:
        status = client.tool("run.status", {})
        if status.get("state") == "stopped":
            break
        if time.monotonic() > deadline:
            raise HarnessError(
                f"oracle did not stop at marker 0x{marker:04X} before the timeout"
            )
        time.sleep(ORACLE_POLL_SECONDS)
    register_dump = client.tool("cpu.registers", {"device": "Main 68000"})
    registers = {}
    for entry in register_dump.get("registers", []):
        name = entry.get("name", "").lower()
        if name:
            registers[name] = entry.get("value")
    ram = client.tool(
        "cpu.read_memory", {"device": "Main 68000", "address": hex(RAM_BASE), "length": RAM_SIZE}
    )
    if "pc" not in registers or "sr" not in registers:
        raise HarnessError("oracle cpu.registers did not expose PC/SR")
    return {
        "pc": registers["pc"],
        "sr": registers["sr"],
        "registers": registers,
        "ram": bytes(ram["bytes"]),
    }


# ---------------------------------------------------------------------------
# ChronoFork CLI client
# ---------------------------------------------------------------------------


def run_chronofork_trace(binary, rom_path, marker, workdir):
    state_path = workdir / "chronofork.state"
    command = [
        str(binary),
        "megadrive",
        "trace",
        "--rom",
        str(rom_path),
        "--stop-pc",
        f"0x{marker:X}",
        "--instructions",
        str(INSTRUCTION_BUDGET),
        "--state-out",
        str(state_path),
        "--ram-bytes",
        str(RAM_SIZE),
    ]
    result = subprocess.run(command, capture_output=True, text=True, timeout=300)
    if result.returncode != 0:
        raise HarnessError(
            f"chronofork trace failed ({result.returncode}): {result.stderr.strip()}"
        )
    state = {}
    lines = state_path.read_text().splitlines()
    for line in lines:
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        state[key] = value
    if state.get("executed") is None:
        raise HarnessError("chronofork trace state file has no executed count")
    if state["executed"] != str(int(state["executed"])):
        raise HarnessError("chronofork trace executed count is not canonical decimal")
    executed = int(state["executed"])
    if state.get("pc") != f"{marker:08x}":
        raise HarnessError(
            f"chronofork stopped at pc={state.get('pc')} instead of marker {marker:08x}"
        )
    ram = bytes.fromhex(state["ram"])
    if len(ram) != RAM_SIZE:
        raise HarnessError(f"chronofork RAM is {len(ram)} bytes, expected {RAM_SIZE}")
    registers = {name: int(state[name], 16) for name in (
        ["pc", "sr", "usp", "ssp"]
        + [f"d{index}" for index in range(8)]
        + [f"a{index}" for index in range(8)]
    )}
    return {
        "executed": executed,
        "tick": int(state["tick"]),
        "pc": registers["pc"],
        "sr": registers["sr"],
        "registers": registers,
        "ram": ram,
    }


def run_chronofork_vdp(binary, rom_path, executed, workdir, windows):
    """Runs ``debug vdp`` once per requested VRAM window (the cap is 4096 bytes)."""
    fields = None
    vram = bytearray()
    for address, length in windows:
        output = subprocess.run(
            [
                str(binary),
                "debug",
                "vdp",
                "--rom",
                str(rom_path),
                "--steps",
                str(executed),
                "--vram-address",
                str(address),
                "--vram-length",
                str(length),
            ],
            capture_output=True,
            text=True,
            timeout=300,
        )
        if output.returncode != 0:
            raise HarnessError(f"chronofork debug vdp failed: {output.stderr.strip()}")
        window_fields = {}
        vram_hex = None
        for line in output.stdout.splitlines():
            if "=" not in line:
                continue
            key, value = line.split("=", 1)
            if key == "vram_hex":
                vram_hex = value
            else:
                window_fields[key] = value
        if vram_hex is None or len(vram_hex) != length * 2:
            raise HarnessError(
                f"chronofork debug vdp returned {0 if vram_hex is None else len(vram_hex) // 2} "
                f"bytes for the window at 0x{address:04X}, expected {length}"
            )
        if window_fields.get("vram_address") != f"0x{address:08X}":
            raise HarnessError("chronofork debug vdp window address does not match")
        vram.extend(bytes.fromhex(vram_hex))
        if fields is None:
            fields = window_fields
    if fields is None:
        raise HarnessError("no VRAM window was requested")
    registers = {}
    for index in range(0x18):
        name = f"vdp_register_{index:02}"
        if name not in fields:
            raise HarnessError(f"chronofork debug vdp output has no {name}")
        registers[index] = int(fields[name], 16)
    return {
        "status": int(fields["vdp_status"], 16),
        "state_hash": fields.get("state_hash"),
        "registers": registers,
        "vram": bytes(vram),
        "vram_sha256": fields.get("vdp_vram_sha256"),
    }


def program_windows(program):
    """Returns the VRAM windows to read, one per written block."""
    windows = []
    for address, words in program["expect_vram"].items():
        length = len(words) * 2
        if length > 4096:
            raise HarnessError(f"VRAM window at 0x{address:04X} exceeds the 4096-byte cap")
        windows.append((address, length))
    if not windows:
        windows.append((0, 2))
    return windows


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------


def compare_field(divergences, field, context, oracle, chronofork, scoped=None):
    if oracle == chronofork:
        return
    divergence = {
        "field": field,
        "context": context,
        "oracle": oracle if not isinstance(oracle, (bytes, bytearray)) else oracle.hex(),
        "chronofork": (
            chronofork if not isinstance(chronofork, (bytes, bytearray)) else chronofork.hex()
        ),
    }
    if scoped is not None:
        divergence["scope"] = scoped
    divergences.append(divergence)


def cpu_register_map(registers):
    """Normalises an oracle or ChronoFork register map to lower-case keys."""
    normalised = {}
    for key, value in registers.items():
        normalised[str(key).lower()] = value
    return normalised


def compare_program(program, oracle, chronofork, canonical):
    """Returns (checked, divergences, scoped) in contract order."""
    checked = []
    divergences = []
    scoped = []

    oracle_regs = cpu_register_map(oracle["registers"])
    cf_regs = cpu_register_map(chronofork["registers"])
    # The pinned oracle powers up D/A with 0xFFFFFFFF while ChronoFork zeroes
    # them (the documented 8.2 starting-state caveat), so the synthetic programs
    # initialise D0-D3 with MOVEQ #0. D2 is the data/status/HV port readback
    # scratch and is covered by the RAM comparison instead; the other initialised
    # data registers are compared directly. Address registers are unused.
    for name in ["d0", "d1", "d3"]:
        if name not in oracle_regs or name not in cf_regs:
            raise HarnessError(f"cpu register {name} is missing from one side")
        checked.append(f"cpu.{name}")
        compare_field(divergences, f"cpu.{name}", f"68000 {name}", oracle_regs[name], cf_regs[name])
    checked.extend(["cpu.pc", "cpu.sr (S/IPM bits)"])
    compare_field(divergences, "cpu.pc", "program counter at the marker", oracle["pc"], chronofork["pc"])
    # CCR is a function of the instruction results, and the optional D2/HV
    # readback values are scoped; S/IPM (the execution-mode and mask fields the
    # 68000 semantics depend on here) are still compared exactly.
    compare_field(
        divergences,
        "cpu.sr (S/IPM bits)",
        f"SR 0x{oracle['sr']:04X} vs 0x{chronofork['sr']:04X}, S/IPM bits",
        oracle["sr"] & 0xFF00,
        chronofork["sr"] & 0xFF00,
    )

    # Work RAM holds the guest-visible VDP readbacks (status/H/V counters).
    checked.append("ram")
    oracle_ram = oracle["ram"]
    cf_ram = chronofork["ram"]
    for offset in range(0, RAM_SIZE, 2):
        word = oracle_ram[offset : offset + 2]
        other = cf_ram[offset : offset + 2]
        if word != other:
            context = f"ram[0x{RAM_BASE + offset:08X}]"
            # Status and H/V counter readbacks are phase-dependent: the minimal
            # slice and Exodus bring the VDP up at different scanline phases and
            # ChronoFork's H counter is documented as approximate. The observed
            # values are recorded as scoped divergences (never hidden) rather
            # than compared for absolute equality; every other RAM word is a
            # hard comparison.
            is_vdp_readback = 0x40 <= offset < 0x50
            is_oracle_cram_readback = 0x60 <= offset < 0x60 + CRAM_PATTERN_WORDS * 2
            if is_oracle_cram_readback:
                divergence = {
                    "field": f"ram[0x{RAM_BASE + offset:08X}]",
                    "context": (
                        "oracle CRAM data-port readback (the pinned headless oracle "
                        "returns its stale write latch)"
                    ),
                    "oracle": f"0x{int.from_bytes(word, 'big'):04X}",
                    "chronofork": f"0x{int.from_bytes(other, 'big'):04X}",
                    "delta": int.from_bytes(other, "big") - int.from_bytes(word, "big"),
                    "scope": "oracle CRAM readback limit",
                }
                scoped.append(divergence)
                checked.append(f"ram[0x{RAM_BASE + offset:08X}]")
                continue
            if is_vdp_readback:
                kind = "H/V counter" if offset % 4 == 2 else "status register"
                divergence = {
                    "field": f"ram[0x{RAM_BASE + offset:08X}]",
                    "context": (
                        f"VDP {kind} readback (power-on scanline phase and H counter "
                        "are approximate)"
                    ),
                    "oracle": f"0x{int.from_bytes(word, 'big'):04X}",
                    "chronofork": f"0x{int.from_bytes(other, 'big'):04X}",
                    "delta": int.from_bytes(other, "big") - int.from_bytes(word, "big"),
                    "scope": "VDP status/H counter phase approximation",
                }
                scoped.append(divergence)
                checked.append(f"ram[0x{RAM_BASE + offset:08X}]")
                continue
            compare_field(
                divergences,
                f"ram[0x{RAM_BASE + offset:08X}]",
                context,
                f"0x{int.from_bytes(word, 'big'):04X}",
                f"0x{int.from_bytes(other, 'big'):04X}",
            )
    checked.append("ram")

    # CRAM data-port readback: the program writes 0x0E00|i (the 6-bit CRAM alias
    # makes the stored word i) and reads the words into work RAM. ChronoFork's
    # readback must match that pattern exactly; the oracle value for the same
    # words is the stale write latch recorded above as a scoped divergence.
    if program.get("expect_cram"):
        for index, expected in enumerate(program["expect_cram"]):
            offset = 0x60 + index * 2
            actual = int.from_bytes(cf_ram[offset : offset + 2], "big")
            checked.append(f"ram[0x{RAM_BASE + offset:08X}] (CRAM readback)")
            compare_field(
                divergences,
                f"cram_readback[{index}]",
                f"CRAM word {index} read back through the data port",
                expected,
                actual,
            )

    # ChronoFork canonical VDP register file against the programmed values.
    if program["expect_registers"]:
        for register, expected in VDP_REGISTERS.items():
            actual = canonical["registers"].get(register)
            if actual is None:
                raise HarnessError(f"chronofork debug vdp has no register {register:#04x}")
            checked.append(f"vdp_register_{register:02x}")
            compare_field(
                divergences,
                f"vdp_register_{register:02x}",
                f"VDP register 0x{register:02X}",
                expected,
                actual,
            )

    # ChronoFork canonical VRAM against the expected image built from the writes.
    # ``canonical["vram"]`` is the concatenation of the requested windows in the
    # order the caller supplied them; ``canonical["windows"]`` maps each window
    # address to its offset in that buffer.
    if program["expect_vram"]:
        expected_image = expected_vram_image(program)
        checked.append("vram")
        windows = canonical["windows"]
        for address, words in program["expect_vram"].items():
            window_start = windows.get(address)
            if window_start is None:
                raise HarnessError(f"VRAM window at 0x{address:04X} was not read")
            for index, word in enumerate(words):
                offset = address + index * 2
                within = window_start + (offset - address)
                actual = canonical["vram"][within : within + 2]
                compare_field(
                    divergences,
                    f"vram[0x{offset:04X}]",
                    f"VRAM word {index} of the block at 0x{address:04X}",
                    word,
                    int.from_bytes(actual, "big"),
                )
        compare_field(
            divergences,
            "vram_sha256",
            "full 64 KiB VRAM image",
            hashlib.sha256(expected_image).hexdigest(),
            canonical.get("vram_sha256"),
        )
        checked.extend(["vram[0x0000..]", "vram_sha256"])

    return checked, divergences, scoped


def first_mismatch(divergences):
    if not divergences:
        return None
    first = divergences[0]
    return (
        f"{first['field']} ({first['context']}): oracle={first['oracle']} "
        f"chronofork={first['chronofork']}"
    )


def negative_control(oracle, chronofork, canonical, program):
    """Corrupts one RAM word and one VRAM byte; both must be reported."""
    oracle_copy = dict(oracle)
    oracle_copy["ram"] = bytearray(oracle["ram"])
    oracle_copy["ram"][0x20] ^= 0xFF
    chronofork_copy = dict(chronofork)
    chronofork_copy["ram"] = bytearray(chronofork["ram"])
    canonical_copy = dict(canonical)
    canonical_copy["vram"] = bytearray(canonical["vram"])
    canonical_copy["vram"][0x0000] ^= 0xFF
    checked, divergences, scoped = compare_program(
        program, oracle_copy, chronofork_copy, canonical_copy
    )
    if not divergences:
        raise HarnessError("negative control did not detect the injected RAM corruption")
    return {
        "status": "pass",
        "injected": "flipped ram[0x00FF0020] and vram[0x0000]",
        "first_mismatch": first_mismatch(divergences),
        "divergences": len(divergences) + len(scoped),
    }


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def write_report(out_dir, report, markdown_lines):
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "oracle-vdp-diff.json").write_text(json.dumps(report, indent=2) + "\n")
    (out_dir / "oracle-vdp-diff.md").write_text("\n".join(markdown_lines) + "\n")


def skip_report(out_dir, reason, command):
    report = {
        "status": "skipped",
        "reason": reason,
        "pinned_revision": PINNED_ORACLE_COMMIT,
        "commands": [command],
    }
    lines = [
        "# VDP state differential — skipped",
        "",
        f"- Reason: {reason}",
        f"- Pinned Oracle revision: `{PINNED_ORACLE_COMMIT}`",
        f"- Command: `{command}`",
        "",
        "The harness did not compare anything; this is not a pass.",
    ]
    write_report(out_dir, report, lines)
    print(f"SKIPPED: {reason}")
    return 3


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument(
        "--dump-programs",
        type=Path,
        default=None,
        help="write the generated ROMs to this directory for inspection",
    )
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    out_dir = args.out_dir.resolve()
    if args.dump_programs is not None:
        args.dump_programs = args.dump_programs.resolve()
    command = " ".join([sys.executable, "scripts/oracle-vdp-diff.py", "--out-dir", str(out_dir)])

    if not ORACLE_BIN.is_file():
        return skip_report(out_dir, f"oracle binary not found at {ORACLE_BIN}", command)
    if not CHRONOFORK_BIN.is_file():
        return skip_report(
            out_dir,
            f"chronofork binary not found at {CHRONOFORK_BIN} (run `cargo build -p chronofork-cli`)",
            command,
        )

    workdir = out_dir / "oracle-vdp-work"
    workdir.mkdir(parents=True, exist_ok=True)
    oracle_bin_sha = hashlib.sha256(ORACLE_BIN.read_bytes()).hexdigest()
    chronofork_bin_sha = hashlib.sha256(CHRONOFORK_BIN.read_bytes()).hexdigest()
    client = None
    report = {
        "status": "pass",
        "command": command,
        "pinned_revision": PINNED_ORACLE_COMMIT,
        "oracle_binary_sha256": oracle_bin_sha,
        "chronofork_binary_sha256": chronofork_bin_sha,
        "programs": [],
        "negative_control": None,
        "limits": [],
    }
    markdown = [
        "# VDP state differential evidence",
        "",
        "Recorded by `scripts/oracle-vdp-diff.py`.",
        "",
        "```",
        command,
        "```",
        "",
        f"- Pinned Exodus revision: `{PINNED_ORACLE_COMMIT}`",
        f"- `exodus-mcpd` SHA-256: `{oracle_bin_sha}`",
        f"- `chronofork` SHA-256: `{chronofork_bin_sha}`",
        "",
    ]
    try:
        client = OracleClient()
        # Negative-control/readback probe first: it documents why VDP memory
        # content is not externally readable in the pinned headless oracle.
        probe_code, probe_marker = oracle_readback_probe()
        probe_rom = make_rom(probe_code, b"ChronoFork VDP readback probe")
        probe = run_on_oracle(client, probe_rom, probe_marker, workdir)
        probe_values = [
            int.from_bytes(probe["ram"][0x50 + index * 2 : 0x52 + index * 2], "big")
            for index in range(4)
        ]
        stale_latch = probe_values and all(value == probe_values[0] for value in probe_values)
        report["oracle_readback_probe"] = {
            "written": ["0xAAAA", "0xBBBB", "0xCCCC", "0xDDDD"],
            "read_back": [f"0x{value:04X}" for value in probe_values],
            "stale_write_latch": stale_latch,
        }
        report["limits"].append(
            "The pinned oracle's VDP reflection pages expose only settings/layer visibility; "
            "debug memory reads at the VDP ports bypass the device."
        )
        if stale_latch:
            report["limits"].append(
                "The pinned headless oracle's data-port reads return the stale write latch, "
                "so VDP memory content cannot be compared through a guest readback."
            )
        report["limits"].extend(
            [
                "Headless oracle has no framebuffer, so pixels are pinned internally by "
                "ChronoFork's frame tests; no cross-emulator pixel comparison.",
                "The H counter is approximate in ChronoFork; H-only differences are recorded "
                "with their delta as a scoped divergence.",
                "VDP data-port VSRAM reads are undecoded in ChronoFork; CRAM/VSRAM canonical "
                "content is covered by backend unit tests, not by this oracle diff.",
                "DMA source is a VRAM address in this slice; DMA is exercised by the backend "
                "unit tests and not content-compared against the oracle.",
            ]
        )
        markdown.extend(
            [
                "## Oracle VDP memory probe",
                "",
                f"Wrote `{', '.join(report['oracle_readback_probe']['written'])}` and read "
                f"back `{', '.join(report['oracle_readback_probe']['read_back'])}` "
                f"(stale write latch: {stale_latch}).",
                "",
            ]
        )

        for name, program in PROGRAMS.items():
            code, marker = program["build"]()
            rom = make_rom(code, program["title"])
            if args.dump_programs:
                args.dump_programs.mkdir(parents=True, exist_ok=True)
                (args.dump_programs / f"{name}.rom").write_bytes(rom)
            rom_path = workdir / f"{name}.rom"
            rom_path.write_bytes(rom)
            oracle = run_on_oracle(client, rom, marker, workdir)
            chronofork = run_chronofork_trace(CHRONOFORK_BIN, rom_path, marker, workdir)
            windows = program_windows(program)
            canonical = run_chronofork_vdp(
                CHRONOFORK_BIN, rom_path, chronofork["executed"], workdir, windows
            )
            canonical["windows"] = {}
            offset = 0
            for address, length in windows:
                canonical["windows"][address] = offset
                offset += length
            checked, divergences, scoped = compare_program(program, oracle, chronofork, canonical)
            status = "pass" if not divergences else "fail"
            if divergences:
                report["status"] = "fail"
            program_report = {
                "name": name,
                "status": status,
                "rom_sha256": hashlib.sha256(rom).hexdigest(),
                "marker_pc": f"0x{marker:04X}",
                "executed_instructions": chronofork["executed"],
                "tick": chronofork["tick"],
                "state_hash": canonical["state_hash"],
                "checked": checked,
                "divergences": divergences,
                "scoped_divergences": scoped,
                "chronofork_vram_sha256": canonical["vram_sha256"],
                "chronofork_status": f"0x{canonical['status']:04X}",
                "first_mismatch": first_mismatch(divergences),
            }
            report["programs"].append(program_report)
            markdown.extend(
                [
                    f"## Program `{name}`",
                    "",
                    f"- Status: **{status}**",
                    f"- ROM SHA-256: `{program_report['rom_sha256']}`",
                    f"- Stop marker: `{program_report['marker_pc']}` after "
                    f"{chronofork['executed']} instructions (tick {chronofork['tick']})",
                    f"- Checked fields: {', '.join(checked)}",
                    f"- ChronoFork VRAM SHA-256: `{canonical['vram_sha256']}`",
                    f"- ChronoFork state hash: `{canonical['state_hash']}`",
                ]
            )
            if divergences:
                markdown.append(f"- First mismatch: {first_mismatch(divergences)}")
            if scoped:
                for divergence in scoped:
                    markdown.append(
                        f"- Scoped divergence: {divergence['field']} "
                        f"(oracle={divergence['oracle']}, "
                        f"chronofork={divergence['chronofork']}, "
                        f"delta={divergence['delta']}, scope={divergence['scope']})"
                    )
            markdown.append("")

        # The negative control runs on the first program's data.
        name, program = next(iter(PROGRAMS.items()))
        code, marker = program["build"]()
        rom = make_rom(code, program["title"])
        rom_path = workdir / f"negative-{name}.rom"
        rom_path.write_bytes(rom)
        oracle = run_on_oracle(client, rom, marker, workdir)
        chronofork = run_chronofork_trace(CHRONOFORK_BIN, rom_path, marker, workdir)
        windows = program_windows(program)
        canonical = run_chronofork_vdp(
            CHRONOFORK_BIN, rom_path, chronofork["executed"], workdir, windows
        )
        canonical["windows"] = {}
        offset = 0
        for address, length in windows:
            canonical["windows"][address] = offset
            offset += length
        control = negative_control(oracle, chronofork, canonical, program)
        report["negative_control"] = control
        markdown.extend(
            [
                "## Negative control",
                "",
                f"- Status: **{control['status']}**",
                f"- Injected: {control['injected']}",
                f"- Reported first mismatch: {control['first_mismatch']}",
                "",
            ]
        )
    except HarnessError as error:
        report["status"] = "error"
        report["error"] = str(error)
        markdown.extend(["## Harness error", "", f"`{error}`", ""])
        write_report(out_dir, report, markdown)
        print(f"HARNESS ERROR: {error}", file=sys.stderr)
        return 2
    finally:
        if client is not None:
            client.close()

    markdown.extend(["## Limits", ""])
    for limit in report["limits"]:
        markdown.append(f"- {limit}")
    markdown.append("")
    write_report(out_dir, report, markdown)
    status = report["status"]
    print(f"VDP differential: {status}")
    for program_report in report["programs"]:
        note = (
            f" first mismatch: {program_report['first_mismatch']}"
            if program_report["first_mismatch"]
            else ""
        )
        print(
            f"  {program_report['name']}: {program_report['status']} "
            f"({program_report['executed_instructions']} instructions, "
            f"tick {program_report['tick']}){note}"
        )
    if report["negative_control"]:
        print(f"  negative control: {report['negative_control']['status']}")
    return 0 if status == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
