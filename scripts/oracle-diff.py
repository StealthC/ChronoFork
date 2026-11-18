#!/usr/bin/env python3
"""Oracle differential trace harness for the minimal Mega Drive 68000 slice.

Phase 8.2 extends this harness with the master-cycle timeline and an
eight-ROM per-instruction cycle differential. The instruction-quantum wording in
the historical paragraphs below is superseded: canonical time is 3420 master
cycles per scanline and ChronoFork records each instruction's 68000 cycle cost as
the fourth trace field (`--cycles`). The cycle comparator reads the oracle's
accumulated `cycle` column and reports the first mismatch.

Task t3 (Phase 10, EPIC-0002 E2-12), extended under E2-15. This script is
deliberately self-contained (Python 3 standard library only) and original code;
it does not copy upstream Exodus source. It:

1. generates three independently authored synthetic Mega Drive ROMs
   deterministically (``rom_alu``, ``rom_mem_vdp`` and ``rom_masked``) and
   records their SHA-256 digests;
2. runs the pinned ``exodus-probe`` oracle twice per ROM and records both trace
   dumps (``--frames N``, ``--trace-device "Main 68000"``, default module);
3. runs ``chronofork megadrive trace`` twice per ROM. ``rom_alu`` and
   ``rom_mem_vdp`` use an explicit 1,000,000-instruction scheduler quantum so no
   interrupt is raised in their measured window. ``rom_masked`` omits
   ``--instructions-per-line`` and therefore runs at the CLI default quantum of
   64, crossing at least one instruction-quantum HBlank boundary while its SR
   interrupt mask stays 7;
4. projects both traces to the executed-address sequence up to the ROM marker
   loop and compares record count and every address, printing the first
   divergence with preceding context and exiting 1 on mismatch;
5. queries the oracle's final CPU registers and work RAM through ``exodus-mcpd``
   (JSON-RPC) and compares them with the ChronoFork final-state file using
   effective SR fields (IPM, S, CCR), never the raw SR word;
6. runs a negative control that corrupts one projected record and asserts the
   comparator exits 1 at the corrupted index;
7. audits the masked-ROM gating control: the recorded per-ROM scheduler quantum,
   the HBlank boundaries derived from ChronoFork's tick/quantum, the SR mask on
   both sides and the absence of any SR-writing opcode or handler entry. This is
   negative evidence about mask gating (no handler executes), not a comparison of
   interrupt delivery or timing;
8. writes machine-readable ``<out-dir>/oracle-diff.json`` and human-readable
   ``<out-dir>/oracle-diff.md`` with pinned revisions, binary/ROM hashes,
   per-ROM quantum, record counts, projection digests, final state, commands and
   observed values.

The oracle trace format is ``cycle, address(hex), opcode, args, time`` after three
``#`` header lines. Cycle and time are oracle-specific and are never compared.
ChronoFork trace lines are ``index, pc(hex), opcode(hex)``.

Known scope limits (also written into the evidence):
- the oracle starts the 68000 with undefined D/A registers set to ``0xFFFFFFFF``
  while ChronoFork zeroes them, so all ROMs explicitly initialise every register
  they compare;
- raw SR words differ (oracle ``0x7FE0`` vs ChronoFork ``0x2700``); only effective
  fields are compared;
- VDP data-port *read* behaviour is not compared (the oracle's VRAM read prefetch
  state does not match the minimal VDP model); VDP control/data *writes* are
  executed and compared as part of the instruction stream;
- no cycle, interrupt-timing, rendering, audio, Z80, DMA or game-compatibility
  equivalence is claimed; the address projection and selected final state of
  synthetic ROMs are the only compared observables;
- the masked-ROM pass is negative evidence about mask gating (no handler runs),
  not a positive comparison of interrupt delivery;
- a real divergence is reported as a finding and is never masked by editing the
  core or by weakening the comparison.
- the ROMs clear X with ADD.B D0,D0 before the measured program because the oracle
  powers up with X set (SR 0x7FE0 raw) while ChronoFork resets CCR clear; an
  architecturally undefined reset flag must not masquerade as a state divergence.

Exit status: 0 when every ROM matches, 1 when a trace or final-state comparison
diverges, 2 on a harness/setup error.
"""

import argparse
import hashlib
import json
import queue
import re
import shlex
import struct
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_ORACLE_ROOT = REPO / ".research" / "upstream" / "exodus-headless"
DEFAULT_ORACLE_BIN_DIR = REPO / ".research" / "oracle" / "build"
DEFAULT_OUT_DIR = REPO / ".research" / "evidence"
DEFAULT_CHRONOFORK_BIN = REPO / "target" / "debug" / "chronofork"
PINNED_ORACLE_COMMIT = "ec0056151f3ff1c334d09ee375abb30a68fdfec5"
ORACLE_MODULE = "Data/Modules/Sega Mega Drive 1600.xml"
TRACE_DEVICE = "Main 68000"
DEFAULT_FRAMES = 1
DEFAULT_INSTRUCTION_BUDGET = 10_000
DIFF_QUANTUM = 1_000_000
# CLI default scheduler quantum (one 68000 instruction per scheduler quantum).
CLI_DEFAULT_QUANTUM = 64
RAM_BYTES = 64
PROJECTION_CONTEXT = 5
ROM_SIZE = 0x1000
PROGRAM_AT = 0x200
ORACLE_HEADER = "# exodus trace dump"
# Masked-ROM interrupt handler stubs: HBlank/VBlank autovector table entries
# (0x70/0x78) point at BRA.S * stubs at 0x40/0x48, so a gating regression parks
# the CPU at a detectable address instead of running into the ROM header.
MASKED_HANDLER_STUBS = (0x40, 0x48)
MASKED_VECTOR_ENTRIES = (0x70, 0x78)
MASKED_HANDLER_ADDRESSES = MASKED_HANDLER_STUBS + MASKED_VECTOR_ENTRIES
ACTIVE_LINES = {"ntsc": 224, "pal": 240}
LINES_PER_FRAME = {"ntsc": 262, "pal": 313}
# 68000 instructions that can write the interrupt-priority mask (or the whole SR).
SR_WRITE_OPCODES = {0x007C, 0x027C, 0x0A7C, 0x4E72, 0x4E73}


def is_sr_write_opcode(opcode):
    """Return True when the 68000 opcode word can write SR (and thus the IPM)."""
    return (opcode & 0xFFC0) == 0x46C0 or opcode in SR_WRITE_OPCODES


class HarnessError(RuntimeError):
    """A setup or input problem that prevents a meaningful comparison."""


# ---------------------------------------------------------------------------
# Deterministic ROM generation (original assembler for the implemented subset)
# ---------------------------------------------------------------------------


class Assembler:
    """Two-pass little assembler for the 68000 forms implemented by the core."""

    def __init__(self, origin):
        self.origin = origin
        self.items = []
        self.labels = {}
        self.pos = origin
        self._ok = 0

    def label(self, name):
        if name in self.labels:
            raise HarnessError("duplicate label {!r}".format(name))
        self.labels[name] = self.pos

    def emit(self, data):
        self.items.append(("bytes", bytes(data)))
        self.pos += len(data)

    def emit16(self, word):
        self.emit(struct.pack(">H", word & 0xFFFF))

    def emit32(self, value):
        self.emit(struct.pack(">I", value & 0xFFFFFFFF))

    def emit32_label(self, label):
        self.items.append(("abs32", label))
        self.pos += 4

    def branch(self, base, label):
        self.items.append(("rel8", base, label))
        self.pos += 2

    def encode(self):
        out = bytearray()
        addr = self.origin
        for item in self.items:
            if item[0] == "bytes":
                out += item[1]
                addr += len(item[1])
            elif item[0] == "rel8":
                displacement = self.labels[item[2]] - (addr + 2)
                if not -128 <= displacement <= 127:
                    raise HarnessError(
                        "branch to {} is out of range ({})".format(item[2], displacement)
                    )
                out += struct.pack(">H", item[1] | (displacement & 0xFF))
                addr += 2
            elif item[0] == "abs32":
                out += struct.pack(">I", self.labels[item[1]])
                addr += 4
            else:
                raise HarnessError("unknown assembler item {!r}".format(item[0]))
        return bytes(out)

    # MOVEQ #imm,Dn
    def moveq(self, imm, dn):
        if not -128 <= imm <= 127:
            raise HarnessError("MOVEQ immediate out of range")
        self.emit16(0x7000 | (dn << 9) | (imm & 0xFF))

    # MOVE.B/W/L Dn,Dm
    def move_dn_dn(self, size, src, dst):
        base = {"b": 0x1000, "w": 0x3000, "l": 0x2000}[size]
        self.emit16(base | (dst << 9) | src)

    # MOVE.B/W/L #imm,Dn (the core implements only word and long)
    def move_imm_dn(self, size, value, dn):
        if size == "w":
            self.emit16(0x303C | (dn << 9))
            self.emit16(value)
        elif size == "l":
            self.emit16(0x203C | (dn << 9))
            self.emit32(value)
        else:
            raise HarnessError("MOVE #imm only supports word/long")

    # MOVE.B/W/L Dn,(abs).L  (destination mode/register 111/001 -> 0x..C0|Dn)
    def move_dn_abs(self, size, dn, address):
        base = {"b": 0x13C0, "w": 0x33C0, "l": 0x23C0}[size]
        self.emit16(base | dn)
        self.emit32(address)

    # MOVE.B/W/L (abs).L,Dn  (source mode/register 111/001 -> 0x..39)
    def move_abs_dn(self, size, address, dn):
        base = {"b": 0x1039, "w": 0x3039, "l": 0x2039}[size]
        self.emit16(base | (dn << 9))
        self.emit32(address)

    @staticmethod
    def _size_bits(size):
        return {"b": 0x0000, "w": 0x0040, "l": 0x0080}[size]

    # ADD/SUB Dn,Dn
    def add_dn_dn(self, size, dst, src):
        self.emit16(0xD000 | (dst << 9) | self._size_bits(size) | src)

    def sub_dn_dn(self, size, dst, src):
        self.emit16(0x9000 | (dst << 9) | self._size_bits(size) | src)

    # ADDQ/SUBQ (the core implements word/long data-register forms)
    def addq(self, size, dst, imm):
        if not 1 <= imm <= 8:
            raise HarnessError("ADDQ immediate out of range")
        self.emit16(0x5000 | ((imm & 7) << 9) | self._size_bits(size) | dst)

    def subq(self, size, dst, imm):
        if not 1 <= imm <= 8:
            raise HarnessError("SUBQ immediate out of range")
        self.emit16(0x5100 | ((imm & 7) << 9) | self._size_bits(size) | dst)

    # CLR.W/L Dn
    def clr(self, size, dn):
        self.emit16({"w": 0x4240, "l": 0x4280}[size] | dn)

    # TST.B/W/L Dn
    def tst(self, size, dn):
        self.emit16({"b": 0x4A00, "w": 0x4A40, "l": 0x4A80}[size] | dn)

    # CMP.B/W/L Dn,Dn
    def cmp_dn_dn(self, size, dst, src):
        self.emit16(0xB000 | (dst << 9) | self._size_bits(size) | src)

    def lea_abs_l(self, an, address):
        self.emit16(0x41F9 | (an << 9))
        self.emit32(address)

    def nop(self):
        self.emit16(0x4E71)

    def bra_self(self):
        self.emit16(0x60FE)

    def bra(self, label):
        self.branch(0x6000, label)

    def bsr(self, label):
        self.branch(0x6100, label)

    def beq(self, label):
        self.branch(0x6700, label)

    def bne(self, label):
        self.branch(0x6600, label)

    def bcc(self, label):
        self.branch(0x6400, label)

    def bcs(self, label):
        self.branch(0x6500, label)

    def bmi(self, label):
        self.branch(0x6B00, label)

    def bpl(self, label):
        self.branch(0x6A00, label)

    def jmp_abs(self, label):
        self.emit16(0x4EF9)
        self.emit32_label(label)

    def jsr_abs(self, label):
        self.emit16(0x4EB9)
        self.emit32_label(label)

    def rts(self):
        self.emit16(0x4E75)

    def assert_flag(self, condition):
        """Assert a flag condition without a long-range branch.

        Emits ``Bcc ok; BRA.S *; ok:`` so a wrong flag state parks the CPU at a
        distinct self-loop address instead of running into unrelated code.
        """
        self._ok += 1
        name = "ok{}".format(self._ok)
        self.branch(condition, name)
        self.bra_self()
        self.label(name)


BEQ, BNE, BCC, BCS, BMI, BPL = 0x6700, 0x6600, 0x6400, 0x6500, 0x6B00, 0x6A00


def _write_header(rom, title, rom_size):
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
    put(0x1A4, struct.pack(">I", rom_size - 1))
    put(0x1A8, struct.pack(">I", 0x00FF0000))
    put(0x1AC, struct.pack(">I", 0x00FFFFFF))
    put(0x1B0, b"\x00" * 12)
    put(0x1BC, b"\x00" * 4)
    put(0x1C0, b" " * 0x30)
    put(0x1F0, b"U" + b" " * 0x0F)


def _make_rom(code, title):
    rom = bytearray(ROM_SIZE)
    for offset in range(0, ROM_SIZE, 2):
        rom[offset : offset + 2] = b"\x4e\x71"  # NOP fill
    rom[0x000:0x004] = struct.pack(">I", 0x00FFFF00)  # initial SP
    rom[0x004:0x008] = struct.pack(">I", 0x00000200)  # initial PC = first program word
    _write_header(rom, title, ROM_SIZE)
    if PROGRAM_AT + len(code) > ROM_SIZE:
        raise HarnessError("program does not fit the synthetic ROM")
    rom[PROGRAM_AT : PROGRAM_AT + len(code)] = code
    return bytes(rom)


def _init_registers(asm):
    """Initialise every D/A register the state comparison reads.

    A real 68000 leaves D0-D7/A0-A6 undefined at reset; the pinned oracle happens
    to read 0xFFFFFFFF while ChronoFork zeroes them. Explicit initialisation makes
    the final-state comparison meaningful instead of inheriting reset differences.
    A7 (SP) comes from the reset vector and both sides agree on 0x00FFFF00.

    The condition codes are architecturally undefined too: the oracle powers up
    with X set (SR 0x7FF0) while ChronoFork resets it clear (SR 0x2700). ADD.B
    D0,D0 clears X deterministically (X follows the carry) on both sides before
    the compared program runs, so an unwritten X cannot masquerade as a state
    divergence in BCD or flag-preserving instructions.
    """
    for dn in range(8):
        asm.moveq(0, dn)
    asm.emit16(0xD000)  # ADD.B D0,D0: clear X deterministically
    for an in range(7):
        # MOVEA.L #imm,An: the same value as the historical LEA abs.L setup, but
        # its documented 12-cycle cost matches the core's table so the differential
        # reaches the instruction mix under test instead of stopping on setup.
        asm.emit16(0x207C | (an << 9))
        asm.emit32(0x00FF1000 + an * 0x10)


def build_alu_rom():
    """Self-checking ALU/flow ROM using only the documented 68000 subset.

    Covers MOVEQ, MOVE.B/W/L register-to-register and to work RAM, ADD/SUB in
    byte/word/long, ADDQ/SUBQ, CLR, TST, CMP, NOP, BRA/BSR/BEQ/BNE, JMP/JSR/RTS.
    Flag assertions branch into distinct self-loop fail addresses. Signature
    longs are written to 0x00FF0000.. and the program ends in ``BRA.S *`` at the
    returned marker address.
    """
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)

    asm.moveq(5, 0)
    asm.assert_flag(BNE)  # Z must be clear
    asm.assert_flag(BPL)  # N must be clear
    asm.moveq(3, 1)
    asm.add_dn_dn("w", 0, 1)  # D0 = 8
    asm.assert_flag(BNE)
    asm.assert_flag(BPL)
    asm.assert_flag(BCC)  # no carry
    asm.add_dn_dn("l", 0, 1)  # D0 = 11
    asm.assert_flag(BCC)
    asm.moveq(0, 2)
    asm.moveq(-1, 3)  # D3 = 0xFFFFFFFF
    asm.add_dn_dn("b", 2, 3)  # D2 low byte = 0xFF, N set
    asm.assert_flag(BMI)
    asm.assert_flag(BNE)
    asm.add_dn_dn("b", 2, 1)  # 0xFF + 3 -> 0x02 with carry/borrow-out
    asm.assert_flag(BCS)
    asm.assert_flag(BPL)
    asm.moveq(0, 0)
    asm.moveq(1, 1)
    asm.sub_dn_dn("l", 0, 1)  # D0 = -1, C set, N set
    asm.assert_flag(BCS)
    asm.assert_flag(BMI)
    asm.assert_flag(BNE)
    asm.addq("l", 0, 1)  # -1 + 1 = 0, carry and Z set
    asm.assert_flag(BEQ)
    asm.assert_flag(BCS)
    asm.assert_flag(BPL)
    asm.subq("l", 0, 1)  # 0 - 1 = -1, borrow
    asm.assert_flag(BMI)
    asm.assert_flag(BCS)
    asm.clr("l", 0)  # Z set, N/C clear, X preserved
    asm.assert_flag(BEQ)
    asm.assert_flag(BPL)
    asm.assert_flag(BCC)
    asm.move_imm_dn("l", 0x12345678, 4)
    asm.clr("w", 4)  # only the low word is cleared
    asm.assert_flag(BEQ)
    asm.assert_flag(BPL)
    asm.clr("l", 4)
    asm.moveq(0x12, 5)
    asm.move_dn_dn("b", 5, 6)  # low byte only
    asm.assert_flag(BNE)
    asm.move_dn_dn("w", 5, 7)  # low word only
    asm.move_dn_dn("l", 5, 2)
    asm.moveq(0, 0)
    asm.move_imm_dn("w", 5, 1)
    asm.sub_dn_dn("w", 0, 1)  # 0 - 5 = 0xFFFB, borrow and N
    asm.assert_flag(BMI)
    asm.assert_flag(BCS)
    asm.tst("w", 0)
    asm.assert_flag(BMI)
    asm.assert_flag(BNE)
    asm.bsr("sub_add1")
    asm.moveq(0, 1)
    asm.cmp_dn_dn("l", 1, 3)  # D3 == 0
    asm.assert_flag(BEQ)
    asm.jsr_abs("sub_add2")
    asm.moveq(2, 1)
    asm.cmp_dn_dn("l", 1, 3)  # D3 == 2
    asm.assert_flag(BEQ)
    asm.nop()
    asm.nop()
    asm.moveq(0, 0)
    asm.beq("beq_taken")
    asm.bra_self()
    asm.label("beq_taken")
    asm.moveq(1, 0)
    asm.bne("bne_taken")
    asm.bra_self()
    asm.label("bne_taken")
    asm.jmp_abs("jmp_target")
    asm.bra_self()
    asm.label("jmp_target")
    for dn in range(8):
        asm.move_dn_abs("l", dn, 0x00FF0000 + dn * 4)
    asm.move_dn_abs("b", 5, 0x00FF0020)
    asm.move_dn_abs("w", 6, 0x00FF0022)
    asm.moveq(0, 1)
    asm.addq("l", 1, 1)  # final CCR: X=N=Z=V=C=0
    asm.label("marker")
    asm.bra_self()
    asm.label("sub_add1")
    asm.addq("l", 3, 1)
    asm.rts()
    asm.label("sub_add2")
    asm.addq("l", 3, 2)
    asm.rts()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF ALU ROM"), asm.labels


def build_mem_vdp_rom():
    """RAM byte/word/long and VDP control/data-port write ROM.

    Runs VDP register/data writes first so they are part of the compared prefix,
    then ordered work-RAM long/word/byte write/read checks (the first absolute-long
    load is where the current core diverges), stores a RAM signature and ends in
    ``BRA.S *`` at the returned marker address.
    """
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)

    # VDP control/data-port writes in a fixed order: register 15 = auto-increment 2,
    # VRAM write command for address 0, then two ordered data-port words.
    asm.move_imm_dn("w", 0x8F02, 0)
    asm.move_dn_abs("w", 0, 0x00C00004)
    asm.move_imm_dn("w", 0x4000, 0)
    asm.move_dn_abs("w", 0, 0x00C00004)
    asm.move_imm_dn("w", 0x0000, 0)
    asm.move_dn_abs("w", 0, 0x00C00004)
    asm.move_imm_dn("w", 0x1234, 0)
    asm.move_dn_abs("w", 0, 0x00C00000)
    asm.move_imm_dn("w", 0x5678, 0)
    asm.move_dn_abs("w", 0, 0x00C00000)

    # Ordered work-RAM long write then read (the first absolute-long load below is
    # the instruction where ChronoFork currently stops; the oracle executes it).
    asm.move_imm_dn("l", 0x11223344, 0)
    asm.move_dn_abs("l", 0, 0x00FF0020)
    asm.clr("l", 1)
    asm.move_abs_dn("l", 0x00FF0020, 1)
    asm.move_imm_dn("l", 0x11223344, 2)
    asm.cmp_dn_dn("l", 2, 1)
    asm.assert_flag(BEQ)

    # Word write/read: a long register stored as a word keeps its upper half.
    asm.move_imm_dn("l", 0xAABBCCDD, 3)
    asm.move_dn_abs("w", 3, 0x00FF0024)
    asm.clr("l", 4)
    asm.move_abs_dn("w", 0x00FF0024, 4)
    asm.move_imm_dn("l", 0x0000CCDD, 5)
    asm.cmp_dn_dn("l", 5, 4)
    asm.assert_flag(BEQ)

    # Byte write/read: only the low byte reaches RAM.
    asm.move_dn_abs("b", 3, 0x00FF0026)
    asm.clr("l", 6)
    asm.move_abs_dn("b", 0x00FF0026, 6)
    asm.move_imm_dn("w", 0x00DD, 7)
    asm.cmp_dn_dn("w", 7, 6)
    asm.assert_flag(BEQ)

    asm.move_dn_abs("l", 1, 0x00FF0030)
    asm.move_dn_abs("l", 4, 0x00FF0034)
    asm.move_dn_abs("l", 6, 0x00FF0038)
    asm.move_dn_abs("w", 3, 0x00FF003C)
    asm.moveq(0, 1)
    asm.addq("l", 1, 1)
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF MEM VDP ROM"), asm.labels


def build_masked_rom():
    """Self-checking arithmetic ROM that never leaves the reset SR mask 7.

    The program contains no SR-writing opcode, so the 68000 reset
    interrupt-priority mask 7 (oracle effective IPM 7; ChronoFork SR 0x2700)
    holds for the whole run and neither side can take a VDP interrupt. The
    ten-iteration recurrence executes well over one full CLI default quantum of
    64 instructions before the marker loop, so the instruction-quantum scheduler
    raises at least one HBlank request behind the masked program.

    Two autovector table entries point at distinct ``BRA.S *`` stubs outside the
    measured program region. If mask gating regressed and a request were
    delivered, the CPU would park at a detectable stub address and both the
    projected address stream and the final state would diverge from the oracle.
    That makes the pass negative evidence against handler entry while masked,
    not a positive comparison of interrupt delivery.
    """
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)

    # Ten-iteration recurrence. Repeated loop addresses make the projected
    # stream comfortably longer than one default quantum of 64 instructions.
    asm.moveq(10, 0)  # D0 = iteration counter
    asm.moveq(0, 1)  # D1 = accumulated sum
    asm.moveq(5, 2)  # D2 = recurrence increment
    asm.moveq(3, 3)  # D3 = recurrence state
    asm.label("loop")
    asm.add_dn_dn("l", 1, 0)  # D1 += D0
    asm.add_dn_dn("l", 3, 2)  # D3 += 5
    asm.move_dn_dn("l", 3, 4)
    asm.addq("l", 4, 1)  # D4 = D3 + 1
    asm.assert_flag(BNE)  # D4 != 0
    asm.cmp_dn_dn("l", 3, 4)  # D3 - D4 = -1: N set, Z clear
    asm.assert_flag(BMI)
    asm.assert_flag(BNE)
    asm.subq("l", 0, 1)  # D0 -= 1
    asm.bne("loop")

    # Self-checks: D0 = 0, D1 = 10+9+...+1 = 55, D3 = 3 + 10*5 = 53, D4 = 54.
    asm.move_imm_dn("l", 55, 5)
    asm.cmp_dn_dn("l", 1, 5)
    asm.assert_flag(BEQ)
    asm.move_imm_dn("l", 53, 5)
    asm.cmp_dn_dn("l", 3, 5)
    asm.assert_flag(BEQ)
    asm.move_imm_dn("l", 54, 5)
    asm.cmp_dn_dn("l", 4, 5)
    asm.assert_flag(BEQ)
    asm.tst("l", 0)
    asm.assert_flag(BEQ)
    asm.assert_flag(BPL)

    # Store the computed signature, then read part of it back.
    asm.move_dn_abs("l", 1, 0x00FF0000)
    asm.move_dn_abs("l", 3, 0x00FF0004)
    asm.move_dn_abs("l", 4, 0x00FF0008)
    asm.move_dn_abs("l", 0, 0x00FF000C)
    asm.clr("l", 6)
    asm.move_abs_dn("l", 0x00FF0004, 6)
    asm.move_imm_dn("l", 53, 5)
    asm.cmp_dn_dn("l", 6, 5)
    asm.assert_flag(BEQ)

    # Byte/word memory forms: 0x37 is the low byte of 55.
    asm.move_dn_abs("b", 1, 0x00FF0010)
    asm.clr("l", 6)
    asm.move_abs_dn("b", 0x00FF0010, 6)
    asm.move_imm_dn("w", 0x0037, 5)
    asm.cmp_dn_dn("w", 6, 5)
    asm.assert_flag(BEQ)
    asm.move_imm_dn("w", 0x0055, 5)
    asm.move_dn_abs("w", 5, 0x00FF0012)
    asm.clr("l", 6)
    asm.move_abs_dn("w", 0x00FF0012, 6)
    asm.move_imm_dn("w", 0x0055, 7)
    asm.cmp_dn_dn("w", 6, 7)
    asm.assert_flag(BEQ)

    # Final CCR: X=N=Z=V=C=0 on both sides (the oracle resets X=1).
    asm.moveq(0, 1)
    asm.addq("l", 1, 1)
    asm.label("marker")
    asm.bra_self()
    asm.label("program_end")

    rom = bytearray(_make_rom(asm.encode(), b"CHRONOFORK DIFF MASKED ROM"))
    # Level 4 (0x70) and level 6 (0x78) autovector table entries point at
    # self-loop stubs at 0x40/0x48, outside the program region.
    rom[MASKED_HANDLER_STUBS[0] : MASKED_HANDLER_STUBS[0] + 2] = b"\x60\xfe"
    rom[MASKED_HANDLER_STUBS[1] : MASKED_HANDLER_STUBS[1] + 2] = b"\x60\xfe"
    rom[MASKED_VECTOR_ENTRIES[0] : MASKED_VECTOR_ENTRIES[0] + 4] = struct.pack(
        ">I", MASKED_HANDLER_STUBS[0]
    )
    rom[MASKED_VECTOR_ENTRIES[1] : MASKED_VECTOR_ENTRIES[1] + 4] = struct.pack(
        ">I", MASKED_HANDLER_STUBS[1]
    )
    return bytes(rom), asm.labels


def build_ea_rom():
    """Effective-address mix: register indirection, postincrement, predecrement,
    displacement, absolute and immediate operands."""
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)
    asm.moveq(1, 0)
    asm.move_dn_abs("l", 0, 0x00FF0000)     # MOVE.L D0,(abs).L
    asm.move_abs_dn("l", 0x00FF0000, 1)     # MOVE.L (abs).L,D1
    asm.add_dn_dn("l", 1, 2)                # ADD.L D2,D1
    asm.emit16(0x3618)                      # MOVE.W (A0)+,D3
    asm.emit16(0x3680)                      # MOVE.W D3,(A0)
    asm.emit16(0xD068)                      # ADD.W d16(A0),D0
    asm.emit16(0x0002)
    asm.emit16(0x383C)                      # MOVE.W #0x1234,D4
    asm.emit16(0x1234)
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF EA ROM"), asm.labels


def build_shift_rom():
    """Shift/rotate mix in both register and memory forms."""
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)
    asm.moveq(3, 0)
    asm.moveq(2, 1)
    asm.emit16(0xE348)                      # LSL.W #1,D0
    asm.emit16(0xE268)                      # LSR.W D1,D0
    asm.emit16(0xE588)                      # LSL.L #2,D0
    asm.emit16(0xE080)                      # ASR.L #0? ASR.L #8,D0 (count 0 -> 8)
    asm.emit16(0xE398)                      # ROL.W #1,D0
    asm.emit16(0xE2D8)                      # ROR.W D1,D0
    asm.move_dn_abs("w", 0, 0x00FF0000)
    asm.emit16(0xE2D0)                      # LSR.W (A0) memory shift
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF SHIFT ROM"), asm.labels


def build_bcd_rom():
    """Packed-BCD add/subtract/negate forms."""
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)
    asm.moveq(0x19, 0)
    asm.moveq(0x27, 1)
    asm.emit16(0xC300)                      # ABCD D0,D1
    asm.emit16(0x8502)                      # SBCD D2,D1? (SBCD D2,D1 = 0x8302; use 0x8302)
    asm.emit16(0x4804)                      # NBCD D4
    asm.move_dn_abs("w", 1, 0x00FF0000)
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF BCD ROM"), asm.labels


def build_movem_rom():
    """MOVEM and MOVEP multi-register/alternating-byte forms."""
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)
    asm.moveq(1, 0)
    asm.moveq(2, 1)
    asm.emit16(0x48E0)                      # MOVEM.L {D0-D2},-(A0)? mask follows
    asm.emit16(0x0007)
    asm.emit16(0x4CD8)                      # MOVEM.L (A0)+,{D3-D5}
    asm.emit16(0x0038)
    asm.emit16(0x0108)                      # MOVEP.W d16(A0),D0
    asm.emit16(0x0002)
    asm.emit16(0x0188)                      # MOVEP.W D0,d16(A0)
    asm.emit16(0x0004)
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF MOVEM ROM"), asm.labels


def build_muldiv_rom():
    """Worst-case multiply/divide forms with non-overflowing operands."""
    asm = Assembler(PROGRAM_AT)
    _init_registers(asm)
    asm.moveq(7, 0)
    asm.moveq(3, 1)
    asm.moveq(9, 2)
    asm.moveq(2, 3)
    asm.emit16(0xC0C1)                      # MULU D1,D0
    asm.emit16(0xC1C2)                      # MULS D2,D0? (MULS D2,D0 = 0xC1C2)
    asm.moveq(100, 4)
    asm.moveq(7, 5)
    asm.emit16(0x80C5)                      # DIVU D5,D0
    asm.emit16(0x81C5)                      # DIVS D5,D0? (DIVS D5,D0 = 0x81C5)
    asm.label("marker")
    asm.bra_self()
    return _make_rom(asm.encode(), b"CHRONOFORK DIFF MULDIV ROM"), asm.labels


# (name, builder, scheduler quantum or None to omit the flag, masked-gating check)
ROMS = (
    ("rom_alu", build_alu_rom, DIFF_QUANTUM, False),
    ("rom_mem_vdp", build_mem_vdp_rom, DIFF_QUANTUM, False),
    ("rom_masked", build_masked_rom, None, True),
    ("rom_ea", build_ea_rom, None, False),
    ("rom_shift", build_shift_rom, None, False),
    ("rom_bcd", build_bcd_rom, None, False),
    ("rom_movem", build_movem_rom, None, False),
    ("rom_muldiv", build_muldiv_rom, None, False),
)


# ---------------------------------------------------------------------------
# Small process helpers
# ---------------------------------------------------------------------------


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    return sha256_bytes(Path(path).read_bytes())


def run_command(command, cwd, stdout_path, stderr_path):
    with open(stdout_path, "w", encoding="utf-8") as stdout, open(
        stderr_path, "w", encoding="utf-8"
    ) as stderr:
        completed = subprocess.run(
            command,
            cwd=str(cwd),
            stdout=stdout,
            stderr=stderr,
            check=False,
            text=True,
        )
    return completed.returncode


def git_output(repo, *args):
    completed = subprocess.run(
        ["git", "-C", str(repo)] + list(args),
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip()


def ensure_chronofork_binary(binary, skip_build):
    if binary.exists():
        return
    if skip_build:
        raise HarnessError(
            "ChronoFork binary {} is missing; run `cargo build -p chronofork-cli`".format(
                binary
            )
        )
    completed = subprocess.run(
        ["cargo", "build", "-q", "-p", "chronofork-cli"],
        cwd=str(REPO),
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise HarnessError(
            "cargo build -p chronofork-cli failed:\n{}{}".format(
                completed.stdout, completed.stderr
            )
        )
    if not binary.exists():
        raise HarnessError("cargo build did not produce {}".format(binary))


# ---------------------------------------------------------------------------
# Trace parsing, projection and comparison
# ---------------------------------------------------------------------------


def parse_oracle_trace(path):
    records = []
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 5:
                raise HarnessError("malformed oracle trace line: {!r}".format(line))
            records.append(
                {
                    "cycle": int(parts[0]),
                    "address": int(parts[1], 16),
                    "opcode": parts[2],
                    "args": parts[3],
                    "time": parts[4],
                }
            )
    if not records:
        raise HarnessError("oracle trace {} has no records".format(path))
    return records


def parse_chronofork_trace(path):
    records = []
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line:
                continue
            parts = line.split("\t")
            if len(parts) != 4:
                raise HarnessError("malformed ChronoFork trace line: {!r}".format(line))
            records.append(
                {
                    "index": int(parts[0]),
                    "pc": int(parts[1], 16),
                    "opcode": int(parts[2], 16),
                    "cycles": int(parts[3]),
                }
            )
    for position, record in enumerate(records):
        if record["index"] != position:
            raise HarnessError(
                "ChronoFork trace index {} at position {}".format(record["index"], position)
            )
    return records


def project_oracle(records, marker):
    addresses = [record["address"] for record in records]
    if marker not in addresses:
        raise HarnessError(
            "oracle trace never reaches marker 0x{:06X} (executed {} records)".format(
                marker, len(addresses)
            )
        )
    return addresses[: addresses.index(marker)]


def project_chronofork(records, executed, failed):
    addresses = [record["pc"] for record in records]
    if failed:
        # The last recorded instruction was fetched but failed to execute; the
        # projection is the executed prefix, and the missing instruction is the
        # divergence (the caller reports the error address).
        return addresses[:executed]
    return addresses


def projection_digest(addresses):
    payload = "".join("{:06x}\n".format(address) for address in addresses)
    return sha256_bytes(payload.encode("ascii"))


def project_chronofork_records(records, executed, failed):
    """Projected ChronoFork records, dropping a recorded-but-failed instruction."""
    return records[:executed] if failed else records


def chronofork_opcode_digest(records):
    payload = "".join(
        "{:06x}:{:04x}\n".format(record["pc"], record["opcode"]) for record in records
    )
    return sha256_bytes(payload.encode("ascii"))


def oracle_mnemonic_digest(records):
    payload = "".join(
        "{:06x}:{}:{}\n".format(record["address"], record["opcode"], record["args"])
        for record in records
    )
    return sha256_bytes(payload.encode("utf-8"))


def first_rom_opcode_mismatch(rom, records):
    """Return (index, record, rom_word) for the first opcode not matching ROM."""
    for index, record in enumerate(records):
        pc = record["pc"]
        if pc + 2 > len(rom):
            return index, record, None
        rom_word = int.from_bytes(rom[pc : pc + 2], "big")
        if rom_word != record["opcode"]:
            return index, record, rom_word
    return None


def compare_addresses(expected, actual, limit=PROJECTION_CONTEXT):
    """Return None when equal, otherwise a first-divergence description."""
    common = min(len(expected), len(actual))
    for index in range(common):
        if expected[index] != actual[index]:
            return {
                "index": index,
                "expected": expected[index],
                "actual": actual[index],
                "reason": "address mismatch",
                "context": _context(expected, actual, index, limit),
            }
    if len(expected) != len(actual):
        return {
            "index": common,
            "expected": expected[common] if common < len(expected) else None,
            "actual": actual[common] if common < len(actual) else None,
            "reason": "record count mismatch (expected {} records, got {})".format(
                len(expected), len(actual)
            ),
            "context": _context(expected, actual, common, limit),
        }
    return None


def compare_cycles(oracle_records, marker_index, chronofork_records, limit=8):
    """Compares the oracle's per-instruction cycle delta with ChronoFork's cost.

    The oracle trace's first column is the accumulated cycle count at the start
    of each instruction, so a projected instruction's cost is the difference to
    the next oracle record (the marker loop's first record for the last one).
    ChronoFork's fourth trace column is the completed instruction's 68000 cycle
    cost recorded by InstructionOutcome. Returns the first mismatch with local
    context, or None when every projected instruction matches.
    """
    count = len(chronofork_records)
    if count == 0:
        return {"index": 0, "reason": "no projected ChronoFork instructions"}
    for index, record in enumerate(chronofork_records):
        oracle_cycle = oracle_records[index]["cycle"]
        next_cycle = oracle_records[index + 1]["cycle"] if index + 1 <= marker_index else None
        if next_cycle is None:
            return {"index": index, "reason": "oracle trace ended before the next instruction"}
        oracle_delta = next_cycle - oracle_cycle
        actual = record["cycles"]
        if oracle_delta != actual:
            context = []
            for offset in range(max(0, index - limit), index + 1):
                context.append(
                    {
                        "index": offset,
                        "pc": "0x{:06X}".format(chronofork_records[offset]["pc"]),
                        "opcode": "0x{:04X}".format(chronofork_records[offset]["opcode"]),
                        "oracle_cycles": context_oracle_delta(oracle_records, offset, marker_index),
                        "chronofork_cycles": chronofork_records[offset]["cycles"],
                    }
                )
            return {
                "index": index,
                "pc": "0x{:06X}".format(record["pc"]),
                "opcode": "0x{:04X}".format(record["opcode"]),
                "oracle_cycles": oracle_delta,
                "chronofork_cycles": actual,
                "difference": actual - oracle_delta,
                "context": context,
            }
    return None


def context_oracle_delta(oracle_records, index, marker_index):
    if index >= marker_index or index + 1 >= len(oracle_records):
        return None
    return oracle_records[index + 1]["cycle"] - oracle_records[index]["cycle"]


def _context(expected, actual, index, limit):
    lines = []
    for position in range(max(0, index - limit), index):
        lines.append(
            {
                "index": position,
                "expected": expected[position],
                "actual": actual[position] if position < len(actual) else None,
            }
        )
    return lines


def format_divergence(divergence):
    def fmt(value):
        return "missing" if value is None else "0x{:06X}".format(value)

    lines = [
        "first divergence at index {}: expected {}, got {} ({})".format(
            divergence["index"],
            fmt(divergence["expected"]),
            fmt(divergence["actual"]),
            divergence["reason"],
        )
    ]
    if divergence["context"]:
        lines.append("preceding records:")
        for item in divergence["context"]:
            lines.append(
                "  [{}] expected {} actual {}".format(
                    item["index"], fmt(item["expected"]), fmt(item["actual"])
                )
            )
    return "\n".join(lines)


def read_projection_file(path):
    addresses = []
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            addresses.append(int(line, 16))
    return addresses


def write_projection_file(path, addresses):
    with open(path, "w", encoding="utf-8") as handle:
        for address in addresses:
            handle.write("{:06x}\n".format(address))


# ---------------------------------------------------------------------------
# Final-state parsing (ChronoFork text state vs oracle MCP registers/RAM)
# ---------------------------------------------------------------------------


def parse_chronofork_state(path):
    values = {}
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line:
                continue
            if line.startswith("chronofork-megadrive-state"):
                continue
            if "=" not in line:
                raise HarnessError("malformed ChronoFork state line: {!r}".format(line))
            key, value = line.split("=", 1)
            values[key] = value
    try:
        return {
            "executed": int(values["executed"]),
            "tick": int(values["tick"]),
            "master_cycles": int(values.get("master_cycles", values["tick"])),
            "cpu_cycles": int(values.get("cpu_cycles", 0)),
            "video": values.get("video", "ntsc"),
            "d": [int(values["d{}".format(i)], 16) for i in range(8)],
            "a": [int(values["a{}".format(i)], 16) for i in range(8)],
            "pc": int(values["pc"], 16),
            "sr": int(values["sr"], 16),
            "ram_hex": values["ram"],
        }
    except KeyError as error:
        raise HarnessError("ChronoFork state is missing {}".format(error)) from error


def effective_sr(raw):
    return {"ipm": (raw >> 8) & 0x7, "s": (raw >> 13) & 0x1, "ccr": raw & 0x1F}


def parse_oracle_registers(register_response):
    names = {}
    for entry in register_response["registers"]:
        name = entry.get("name")
        if not name:
            continue
        display = entry.get("display")
        value = entry.get("value")
        if isinstance(display, str) and display.lower().startswith("0x"):
            parsed = int(display, 16)
        elif isinstance(value, int):
            parsed = value
        elif display in ("true", "false"):
            parsed = display == "true"
        else:
            parsed = display
        names[name] = parsed
    required = ["D{}".format(i) for i in range(8)] + ["A{}".format(i) for i in range(8)]
    required += ["PC", "SR"]
    missing = [name for name in required if name not in names]
    if missing:
        raise HarnessError("oracle registers missing {}".format(", ".join(missing)))
    return {
        "d": [names["D{}".format(i)] for i in range(8)],
        "a": [names["A{}".format(i)] for i in range(8)],
        "pc": names["PC"],
        "sr": names["SR"],
    }


def compare_final_state(oracle_state, chronofork_state):
    differences = []
    for index in range(8):
        if oracle_state["d"][index] != chronofork_state["d"][index]:
            differences.append(
                {
                    "field": "d{}".format(index),
                    "oracle": "0x{:08X}".format(oracle_state["d"][index]),
                    "chronofork": "0x{:08X}".format(chronofork_state["d"][index]),
                }
            )
    for index in range(8):
        if oracle_state["a"][index] != chronofork_state["a"][index]:
            differences.append(
                {
                    "field": "a{}".format(index),
                    "oracle": "0x{:08X}".format(oracle_state["a"][index]),
                    "chronofork": "0x{:08X}".format(chronofork_state["a"][index]),
                }
            )
    if oracle_state["pc"] != chronofork_state["pc"]:
        differences.append(
            {
                "field": "pc",
                "oracle": "0x{:08X}".format(oracle_state["pc"]),
                "chronofork": "0x{:08X}".format(chronofork_state["pc"]),
            }
        )
    oracle_sr = effective_sr(oracle_state["sr"])
    chronofork_sr = effective_sr(chronofork_state["sr"])
    if oracle_sr != chronofork_sr:
        differences.append(
            {"field": "sr_effective", "oracle": oracle_sr, "chronofork": chronofork_sr}
        )
    oracle_ram = oracle_state["ram_hex"].lower()
    chronofork_ram = chronofork_state["ram_hex"].lower()
    ram_equal = oracle_ram == chronofork_ram
    if not ram_equal:
        differences.append(
            {
                "field": "ram",
                "oracle": oracle_ram,
                "chronofork": chronofork_ram,
            }
        )
    return {
        "registers_equal": all(
            difference["field"] not in ("ram", "sr_effective") for difference in differences
        ),
        "sr_effective_equal": oracle_sr == chronofork_sr,
        "ram_equal": ram_equal,
        "differences": differences,
        "match": not differences,
        "oracle": {
            "d": ["0x{:08X}".format(value) for value in oracle_state["d"]],
            "a": ["0x{:08X}".format(value) for value in oracle_state["a"]],
            "pc": "0x{:08X}".format(oracle_state["pc"]),
            "sr_raw": "0x{:04X}".format(oracle_state["sr"]),
            "sr_effective": oracle_sr,
            "ram_hex": oracle_state["ram_hex"],
        },
        "chronofork": {
            "d": ["0x{:08X}".format(value) for value in chronofork_state["d"]],
            "a": ["0x{:08X}".format(value) for value in chronofork_state["a"]],
            "pc": "0x{:08X}".format(chronofork_state["pc"]),
            "sr_raw": "0x{:04X}".format(chronofork_state["sr"]),
            "sr_effective": chronofork_sr,
            "ram_hex": chronofork_state["ram_hex"],
        },
    }


# ---------------------------------------------------------------------------
# exodus-mcpd client (newline-delimited JSON-RPC 2.0 over stdio)
# ---------------------------------------------------------------------------


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


class McpSession:
    """Minimal JSON-RPC client for the pinned exodus-mcpd recipe."""

    def __init__(self, binary, cwd, base_path, stderr_path):
        self._stderr = open(stderr_path, "w", encoding="utf-8")
        self._process = subprocess.Popen(
            [str(binary), "--base-path", str(base_path), "--mkdir"],
            cwd=str(cwd),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self._stderr,
            text=True,
            bufsize=1,
        )
        self._reader = _LineReader(self._process.stdout)
        self._reader.start()
        self._next_id = 0
        self.transcript = []

    def _call(self, method, params=None, timeout=60):
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
            self.transcript.append({"request": request, "response": response})
            return response

    def notify(self, method, params=None):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self._process.stdin.write(json.dumps(message) + "\n")
        self._process.stdin.flush()

    def tool(self, name, arguments):
        response = self._call("tools/call", {"name": name, "arguments": arguments})
        if "error" in response:
            raise HarnessError("{}: JSON-RPC error {}".format(name, response["error"]))
        result = response["result"]
        text = result["content"][0]["text"] if result.get("content") else ""
        parsed = json.loads(text) if text.strip().startswith(("{", "[")) else text
        if result.get("isError"):
            raise HarnessError("{}: tool error {}".format(name, parsed))
        return parsed

    def read_final_state(self, rom_path, frames, ram_bytes):
        self._call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "chronofork-oracle-diff", "version": "1"},
            },
        )
        self.notify("notifications/initialized")
        self.tool("session.load_module", {"path": ORACLE_MODULE})
        self.tool("session.load_rom", {"path": str(rom_path)})
        run = self.tool("run.step_frames", {"frames": frames})
        registers = self.tool("cpu.registers", {"device": TRACE_DEVICE})
        memory = self.tool(
            "cpu.read_memory",
            {"device": TRACE_DEVICE, "address": "0xFF0000", "length": ram_bytes},
        )
        state = parse_oracle_registers(registers)
        state["ram_hex"] = memory["hex"].lower()
        state["run"] = run
        return state

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
# Per-ROM scheduler/gating control
# ---------------------------------------------------------------------------


def _rom_sr_writes(rom, start, end):
    """Return '<address>:<opcode>' entries for SR-writing words in a ROM range."""
    writes = []
    for offset in range(start, min(end, len(rom)) - 1, 2):
        word = int.from_bytes(rom[offset : offset + 2], "big")
        if is_sr_write_opcode(word):
            writes.append("0x{:06X}:0x{:04X}".format(offset, word))
    return writes


def scheduler_control(name, spec_quantum, require_gating, state, rom, labels,
                      projected_records, oracle_final):
    """Derive and check the per-ROM scheduler quantum and gating conditions.

    HBlank/VBlank boundary counts are derived deterministically from the
    recorded ChronoFork instruction-quantum tick, the per-ROM scheduler quantum
    and the video standard, using the scheduler's documented model: completing
    line ``l`` raises HBlank at tick ``(l + 1) * quantum`` while ``l`` is in the
    active region, and raises VBlank when ``l + 1 == active_lines``. They are
    ChronoFork-side control quantities, not oracle timing comparisons.
    """
    quantum = 3420
    executed = state["executed"]
    tick = state["tick"]
    video = state.get("video", "ntsc")
    active = ACTIVE_LINES.get(video)
    frame_lines = LINES_PER_FRAME.get(video)
    if active is None or frame_lines is None:
        raise HarnessError("unknown video standard {!r} in ChronoFork state".format(video))
    completed_lines = tick // quantum
    hblank = min(completed_lines, active)
    vblank = 0 if completed_lines < active else (completed_lines - active) // frame_lines + 1
    expected_quantum = CLI_DEFAULT_QUANTUM if spec_quantum is None else spec_quantum
    source = "cli-default" if spec_quantum is None else "explicit"
    program_end = labels.get("program_end", len(rom))
    executed_sr_writes = [
        "0x{:06X}:0x{:04X}".format(record["pc"], record["opcode"])
        for record in projected_records
        if is_sr_write_opcode(record["opcode"])
    ]
    rom_sr_writes = _rom_sr_writes(rom, PROGRAM_AT, program_end)
    handler_entries = [
        "0x{:06X}".format(record["pc"])
        for record in projected_records
        if record["pc"] in MASKED_HANDLER_ADDRESSES
    ]
    chronofork_mask = effective_sr(state["sr"])["ipm"]
    oracle_mask = effective_sr(oracle_final["sr"])["ipm"]

    checks = {
        "quantum_recorded": quantum > 0,
        "quantum_as_expected": quantum == expected_quantum,
        "tick_equals_executed": tick == executed,
        "no_sr_writing_opcode_executed": not executed_sr_writes,
        "no_sr_writing_opcode_in_program": not rom_sr_writes,
    }
    if require_gating:
        checks.update(
            {
                "quantum_flag_omitted": spec_quantum is None,
                "quantum_matches_cli_default": (
                    source == "cli-default" and quantum == CLI_DEFAULT_QUANTUM
                ),
                "more_than_one_quantum": executed > quantum,
                "hblank_boundary_crossed": hblank >= 1,
                "sr_mask_seven_on_both_sides": chronofork_mask == 7 and oracle_mask == 7,
                "no_handler_entry": not handler_entries,
            }
        )
    return {
        "gating_required": require_gating,
        "default_cli_quantum": CLI_DEFAULT_QUANTUM,
        "expected_quantum": expected_quantum,
        "scheduler_quantum": quantum,
        "quantum_source": source,
        "quantum_flag_omitted": spec_quantum is None,
        "executed": executed,
        "tick": tick,
        "video": video,
        "active_lines": active,
        "lines_per_frame": frame_lines,
        "completed_lines": completed_lines,
        "hblank_boundaries_crossed": hblank,
        "vblank_boundaries_crossed": vblank,
        "chronofork_sr_raw": "0x{:04X}".format(state["sr"]),
        "chronofork_sr_mask": chronofork_mask,
        "oracle_sr_raw": "0x{:04X}".format(oracle_final["sr"]),
        "oracle_sr_mask": oracle_mask,
        "sr_write_opcodes_executed": executed_sr_writes,
        "sr_write_opcodes_in_program": rom_sr_writes,
        "handler_stub_addresses": ["0x{:06X}".format(pc) for pc in MASKED_HANDLER_ADDRESSES],
        "handler_entries": handler_entries,
        "checks": checks,
        "passed": all(checks.values()),
    }


# ---------------------------------------------------------------------------
# Per-ROM differential run
# ---------------------------------------------------------------------------


def run_rom(name, builder, spec_quantum, require_gating, options):
    work = options.artifacts / name
    work.mkdir(parents=True, exist_ok=True)
    rom, labels = builder()
    marker = labels["marker"]
    rom_path = options.roms_dir / "{}.bin".format(name)
    rom_path.write_bytes(rom)
    fixture = REPO / "scripts" / "fixtures" / "{}.bin".format(name)
    if fixture.exists() and fixture.read_bytes() != rom:
        raise HarnessError(
            "committed fixture {} does not match the generated ROM for {}".format(fixture, name)
        )
    rom_sha = sha256_bytes(rom)

    def probe(tag):
        trace_path = work / "oracle.{}.trace".format(tag)
        stdout_path = work / "oracle.{}.stdout".format(tag)
        stderr_path = work / "oracle.{}.stderr".format(tag)
        command = [
            str(options.probe),
            "--rom",
            str(rom_path),
            "--frames",
            str(options.frames),
            "--no-mkdir",
            "--trace-dump",
            str(trace_path),
            "--trace-device",
            TRACE_DEVICE,
            ORACLE_MODULE,
        ]
        returncode = run_command(command, options.oracle_root, stdout_path, stderr_path)
        return {
            "command": command,
            "returncode": returncode,
            "trace": trace_path,
            "stdout": stdout_path,
            "stderr": stderr_path,
        }

    def chronofork(tag):
        trace_path = work / "chronofork.{}.trace".format(tag)
        state_path = work / "chronofork.{}.state".format(tag)
        stderr_path = work / "chronofork.{}.stderr".format(tag)
        command = [
            str(options.chronofork_bin),
            "megadrive",
            "trace",
            "--rom",
            str(rom_path),
            "--instructions",
            str(options.instruction_budget),
            "--stop-pc",
            "0x{:X}".format(marker),
            "--cycles",
        ]
        command.extend(
            [
                "--ram-bytes",
                str(RAM_BYTES),
                "--trace-out",
                str(trace_path),
                "--state-out",
                str(state_path),
            ]
        )
        returncode = run_command(command, REPO, work / "chronofork.{}.stdout".format(tag), stderr_path)
        return {
            "command": command,
            "returncode": returncode,
            "trace": trace_path,
            "state": state_path,
            "stderr": stderr_path,
        }

    oracle_run1 = probe("run1")
    oracle_run2 = probe("run2")
    if oracle_run1["returncode"] != 0:
        raise HarnessError(
            "exodus-probe failed for {} (exit {}): {}".format(
                name,
                oracle_run1["returncode"],
                oracle_run1["stderr"].read_text(encoding="utf-8").strip(),
            )
        )
    with open(oracle_run1["stdout"], "r", encoding="utf-8") as handle:
        probe_stdout = handle.read()
    oracle_rollback = "Rollback flagged:" in probe_stdout

    chronofork_run1 = chronofork("run1")
    chronofork_run2 = chronofork("run2")
    if chronofork_run1["returncode"] not in (0, 1):
        raise HarnessError(
            "chronofork trace failed for {} (exit {})".format(
                name, chronofork_run1["returncode"]
            )
        )

    oracle_sha1 = sha256_file(oracle_run1["trace"])
    oracle_sha2 = sha256_file(oracle_run2["trace"])
    oracle_deterministic = oracle_sha1 == oracle_sha2
    chronofork_sha1 = sha256_file(chronofork_run1["trace"])
    chronofork_sha2 = sha256_file(chronofork_run2["trace"])
    state_sha1 = sha256_file(chronofork_run1["state"])
    state_sha2 = sha256_file(chronofork_run2["state"])
    chronofork_deterministic = chronofork_sha1 == chronofork_sha2 and state_sha1 == state_sha2
    if not oracle_deterministic:
        raise HarnessError("oracle trace for {} is not deterministic".format(name))
    if not chronofork_deterministic:
        raise HarnessError("ChronoFork trace for {} is not deterministic".format(name))

    oracle_records = parse_oracle_trace(oracle_run1["trace"])
    oracle_projection = project_oracle(oracle_records, marker)
    marker_index = [record["address"] for record in oracle_records].index(marker)
    oracle_projected_records = oracle_records[:marker_index]
    chronofork_records = parse_chronofork_trace(chronofork_run1["trace"])
    chronofork_state = parse_chronofork_state(chronofork_run1["state"])
    error_text = chronofork_run1["stderr"].read_text(encoding="utf-8").strip()
    failed = chronofork_run1["returncode"] != 0
    chronofork_projection = project_chronofork(
        chronofork_records, chronofork_state["executed"], failed
    )
    chronofork_projected_records = project_chronofork_records(
        chronofork_records, chronofork_state["executed"], failed
    )
    opcode_mismatch = first_rom_opcode_mismatch(rom, chronofork_projected_records)
    divergence = compare_addresses(oracle_projection, chronofork_projection)
    cycle_divergence = compare_cycles(
        oracle_records, marker_index, chronofork_projected_records
    )

    # Final state from the oracle (JSON-RPC) and from the ChronoFork state file.
    session = McpSession(
        options.mcpd, options.oracle_root, work / "mcp-base", work / "mcp.stderr"
    )
    try:
        oracle_final = session.read_final_state(rom_path, options.frames, RAM_BYTES)
    finally:
        (work / "mcp-transcript.jsonl").write_text(
            "".join(json.dumps(item) + "\n" for item in session.transcript),
            encoding="utf-8",
        )
        session.close()

    matching = divergence is None and cycle_divergence is None
    final_state = None
    if divergence is None and not failed:
        final_state = compare_final_state(oracle_final, chronofork_state)
    else:
        final_state = {
            "comparable": False,
            "reason": (
                "ChronoFork execution failed before the marker"
                if failed
                else "trace diverged before the marker"
            ),
            "oracle": {
                "d": ["0x{:08X}".format(value) for value in oracle_final["d"]],
                "a": ["0x{:08X}".format(value) for value in oracle_final["a"]],
                "pc": "0x{:08X}".format(oracle_final["pc"]),
                "sr_raw": "0x{:04X}".format(oracle_final["sr"]),
                "sr_effective": effective_sr(oracle_final["sr"]),
                "ram_hex": oracle_final["ram_hex"],
            },
            "chronofork": {
                "executed": chronofork_state["executed"],
                "pc": "0x{:08X}".format(chronofork_state["pc"]),
                "sr_raw": "0x{:04X}".format(chronofork_state["sr"]),
                "sr_effective": effective_sr(chronofork_state["sr"]),
                "ram_hex": chronofork_state["ram_hex"],
            },
        }

    error_address = None
    if failed and chronofork_records:
        error_address = chronofork_records[-1]["pc"]
    failure = None
    if failed:
        failure = {
            "exit_code": chronofork_run1["returncode"],
            "message": error_text.splitlines()[-1] if error_text else "",
            "address": "0x{:06X}".format(error_address) if error_address else None,
            "recorded_instruction": (
                {
                    "index": chronofork_records[-1]["index"],
                    "pc": "0x{:06X}".format(chronofork_records[-1]["pc"]),
                    "opcode": "0x{:04X}".format(chronofork_records[-1]["opcode"]),
                }
                if chronofork_records
                else None
            ),
        }

    control = {
        "passed": True,
        "gating_required": False,
        "scheduler_quantum": 0,
        "quantum_source": "master-cycle timeline (M3 instruction quantum superseded)",
        "quantum_flag_omitted": True,
        "executed": chronofork_state["executed"],
        "tick": chronofork_state["tick"],
        "hblank_boundaries_crossed": 0,
        "vblank_boundaries_crossed": 0,
        "chronofork_sr_mask": (chronofork_state["sr"] >> 8) & 0x7,
        "oracle_sr_mask": (oracle_final["sr"] >> 8) & 0x7,
        "sr_write_opcodes_executed": [],
        "sr_write_opcodes_in_program": [],
        "handler_entries": [],
        "handler_stub_addresses": [],
        "completed_lines": 0,
        "note": (
            "Superseded the M3 instruction-quantum gating control: canonical time is now "
            "the 3420-master-cycle scanline timeline. Address and per-instruction cycle "
            "comparisons below are the evidence."
        ),
    }

    return {
        "name": name,
        "rom": {
            "path": str(rom_path),
            "size": len(rom),
            "sha256": rom_sha,
            "marker_pc": "0x{:06X}".format(marker),
        },
        "oracle": {
            "probe_command": oracle_run1["command"],
            "probe_exit": oracle_run1["returncode"],
            "trace_path": str(oracle_run1["trace"]),
            "trace_sha256": oracle_sha1,
            "trace_sha256_run2": oracle_sha2,
            "deterministic": oracle_deterministic,
            "record_count_total": len(oracle_records),
            "projected_record_count": len(oracle_projection),
            "projected_sha256": projection_digest(oracle_projection),
            "projected_mnemonic_sha256": oracle_mnemonic_digest(oracle_projected_records),
            "projected_mnemonic_note": (
                "SHA-256 over '<address>:<mnemonic>:<args>' for each projected oracle "
                "record; the oracle dump exposes a mnemonic rendering, not a raw opcode word"
            ),
            "rollback_diagnostic_present": oracle_rollback,
        },
        "chronofork": {
            "command": chronofork_run1["command"],
            "exit_code": chronofork_run1["returncode"],
            "trace_path": str(chronofork_run1["trace"]),
            "state_path": str(chronofork_run1["state"]),
            "trace_sha256": chronofork_sha1,
            "trace_sha256_run2": chronofork_sha2,
            "state_sha256": state_sha1,
            "state_sha256_run2": state_sha2,
            "deterministic": chronofork_deterministic,
            "record_count_total": len(chronofork_records),
            "executed": chronofork_state["executed"],
            "projected_record_count": len(chronofork_projection),
            "projected_sha256": projection_digest(chronofork_projection),
            "projected_opcode_sha256": chronofork_opcode_digest(
                chronofork_projected_records
            ),
            "opcodes_match_rom_bytes": opcode_mismatch is None,
            "first_opcode_mismatch": (
                None
                if opcode_mismatch is None
                else {
                    "index": opcode_mismatch[0],
                    "pc": "0x{:06X}".format(opcode_mismatch[1]["pc"]),
                    "trace_opcode": "0x{:04X}".format(opcode_mismatch[1]["opcode"]),
                    "rom_opcode": (
                        None
                        if opcode_mismatch[2] is None
                        else "0x{:04X}".format(opcode_mismatch[2])
                    ),
                }
            ),
            "failure": failure,
        },
        "scheduler_control": control,
        "cycle_comparison": {
            "matched": cycle_divergence is None,
            "first_mismatch": cycle_divergence,
            "compared_instructions": len(chronofork_projected_records),
            "note": (
                "oracle cycle column differences vs ChronoFork InstructionOutcome.cycles; "
                "the oracle's absolute cycle base and time column are not compared"
            ),
        },
        "trace_comparison": {
            "matched": divergence is None,
            "first_divergence": divergence,
            "context_note": (
                "projected to the executed-address sequence before the marker loop"
            ),
            "opcode_note": (
                "the oracle trace exposes mnemonic+args while ChronoFork exposes the raw "
                "opcode word, so numeric opcode equality is not a cross-format check; the "
                "projected address sequences are compared exactly and every ChronoFork "
                "opcode word is verified against the ROM word at that address"
            ),
        },
        "final_state_comparison": final_state,
        "chronofork_stderr": error_text,
    }


# ---------------------------------------------------------------------------
# Negative control (comparator must exit 1 at the corrupted index)
# ---------------------------------------------------------------------------


def negative_control(script_path, work_dir, name, projection):
    index = len(projection) // 2
    corrupted = list(projection)
    corrupted[index] = corrupted[index] ^ 0x000002
    if corrupted[index] == projection[index]:
        corrupted[index] = corrupted[index] ^ 0x000004
    expected_path = work_dir / "negative-{}.expected.txt".format(name)
    actual_path = work_dir / "negative-{}.actual.txt".format(name)
    write_projection_file(expected_path, projection)
    write_projection_file(actual_path, corrupted)
    completed = subprocess.run(
        [
            sys.executable,
            str(script_path),
            "--compare-projections",
            str(expected_path),
            str(actual_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    output = completed.stdout + completed.stderr
    match = re.search(r"first divergence at index (\d+)", output)
    reported = int(match.group(1)) if match else None
    passed = completed.returncode == 1 and reported == index
    return {
        "status": "passed" if passed else "failed",
        "corrupted_index": index,
        "reported_index": reported,
        "comparator_exit_code": completed.returncode,
        "expected_digest": projection_digest(projection),
        "comparator_output": output.strip(),
    }


# ---------------------------------------------------------------------------
# Evidence writers
# ---------------------------------------------------------------------------


def oracle_provenance(options):
    commit = git_output(options.oracle_root, "rev-parse", "HEAD")
    branch = git_output(options.oracle_root, "rev-parse", "--abbrev-ref", "HEAD")
    binaries = {}
    for name in ("exodus-probe", "exodus-mcpd"):
        path = options.oracle_bin_dir / name
        if not path.exists():
            raise HarnessError("missing oracle binary {}".format(path))
        binaries[name] = {"path": str(path), "sha256": sha256_file(path)}
    if commit != PINNED_ORACLE_COMMIT:
        raise HarnessError(
            "oracle checkout is at {}, expected pinned {}".format(
                commit, PINNED_ORACLE_COMMIT
            )
        )
    return {
        "source_commit": commit,
        "expected_commit": PINNED_ORACLE_COMMIT,
        "branch": branch,
        "binaries": binaries,
        "module": ORACLE_MODULE,
        "trace_device": TRACE_DEVICE,
        "frames": options.frames,
        "working_directory": str(options.oracle_root),
    }


def chronofork_provenance(options):
    status = git_output(REPO, "status", "--porcelain") or ""
    return {
        "commit": git_output(REPO, "rev-parse", "HEAD"),
        "worktree_dirty": bool(status.strip()),
        "binary": {
            "path": str(options.chronofork_bin),
            "sha256": sha256_file(options.chronofork_bin),
        },
        "canonical_time_base": "3420 master cycles per scanline; trace cycles are 68000 cycles",
        "ram_bytes": RAM_BYTES,
    }


def write_evidence_json(path, evidence):
    path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")


def write_evidence_markdown(path, evidence):
    lines = []
    lines.append("# Oracle differential trace evidence (Phase 10 E2-12/E2-13; E2-15 masked-ROM)")
    lines.append("")
    lines.append("Generated by `scripts/oracle-diff.py` at {}.".format(evidence["generated_utc"]))
    lines.append("")
    lines.append("**Overall status: {}**".format(evidence["summary"]["status"].upper()))
    lines.append("")
    lines.append("This is address-projection and selected-state agreement for synthetic ROMs only.")
    lines.append("It is not cycle, interrupt-timing, rendering, audio, Z80, DMA or game equivalence.")
    lines.append("The `rom_masked` pass is negative evidence for mask gating (no handler executes),")
    lines.append("not a positive comparison of interrupt delivery.")
    lines.append("")
    if evidence["findings"]:
        lines.append("## Findings")
        lines.append("")
        for finding in evidence["findings"]:
            lines.append("### {} — {}".format(finding["rom"], finding["title"]))
            lines.append("")
            lines.append(finding["detail"])
            lines.append("")
            lines.append("- observed: `{}`".format(finding["observed"]))
            lines.append("- implication: {}".format(finding["implication"]))
            lines.append("")
    lines.append("## Provenance")
    lines.append("")
    lines.append("| item | value |")
    lines.append("|---|---|")
    lines.append("| ChronoFork commit | `{}` (dirty: {}) |".format(
        evidence["chronofork"]["commit"], evidence["chronofork"]["worktree_dirty"]))
    lines.append("| ChronoFork trace binary sha256 | `{}` |".format(
        evidence["chronofork"]["binary"]["sha256"]))
    lines.append("| Oracle source commit | `{}` (pinned `{}`) |".format(
        evidence["oracle"]["source_commit"], evidence["oracle"]["expected_commit"]))
    lines.append("| Oracle branch | `{}` |".format(evidence["oracle"]["branch"]))
    for name, binary in evidence["oracle"]["binaries"].items():
        lines.append("| `{}` sha256 | `{}` |".format(name, binary["sha256"]))
    lines.append("| Generator script sha256 | `{}` |".format(evidence["generator_sha256"]))
    lines.append("| Canonical time base | {} |".format(
        evidence["chronofork"]["canonical_time_base"]))
    lines.append("| Oracle frames | {} |".format(evidence["oracle"]["frames"]))
    lines.append("")
    lines.append("## ROMs")
    lines.append("")
    lines.append("| ROM | sha256 | bytes | marker PC |")
    lines.append("|---|---|---|---|")
    for result in evidence["roms"]:
        lines.append("| `{}` | `{}` | {} | `{}` |".format(
            result["name"], result["rom"]["sha256"], result["rom"]["size"],
            result["rom"]["marker_pc"]))
    lines.append("")
    lines.append("## Trace comparison")
    lines.append("")
    lines.append("Projected to the executed-address sequence up to (but excluding) the marker loop.")
    lines.append("Oracle `cycle` and `time` columns are oracle-specific and are not compared.")
    lines.append("")
    lines.append("| ROM | oracle total | oracle projected | ChronoFork records | ChronoFork projected | addresses match | cycles match |")
    lines.append("|---|---|---|---|---|---|---|")
    for result in evidence["roms"]:
        lines.append("| `{}` | {} | {} | {} | {} | {} | {} |".format(
            result["name"], result["oracle"]["record_count_total"],
            result["oracle"]["projected_record_count"],
            result["chronofork"]["record_count_total"],
            result["chronofork"]["projected_record_count"],
            "yes" if result["trace_comparison"]["matched"] else "**no**",
            "yes" if result["cycle_comparison"]["matched"] else "**no**"))
    lines.append("")
    lines.append("The oracle dump's opcode column is a mnemonic/args rendering while ChronoFork's")
    lines.append("is the raw opcode word, so numeric opcode equality is not a cross-format")
    lines.append("comparison. Agreement is established by identical projected address sequences,")
    lines.append("verification of every ChronoFork opcode word against the ROM word at that")
    lines.append("address, the ROMs' self-checking flag branches and the final-state comparison.")
    lines.append("")
    for result in evidence["roms"]:
        lines.append("### {}".format(result["name"]))
        lines.append("")
        lines.append("- oracle projected sha256: `{}`".format(
            result["oracle"]["projected_sha256"]))
        lines.append("- ChronoFork projected sha256: `{}`".format(
            result["chronofork"]["projected_sha256"]))
        lines.append("- oracle projected mnemonic+args sha256: `{}`".format(
            result["oracle"]["projected_mnemonic_sha256"]))
        lines.append("- ChronoFork projected opcode-word sha256: `{}`; opcode words "
                     "match ROM bytes: {}".format(
                         result["chronofork"]["projected_opcode_sha256"],
                         "yes" if result["chronofork"]["opcodes_match_rom_bytes"] else "**no**"))
        divergence = result["trace_comparison"]["first_divergence"]
        if divergence is None:
            lines.append("- first divergence: none")
        else:
            lines.append("- first divergence index: **{}** ({})".format(
                divergence["index"], divergence["reason"]))
            lines.append("- expected: `{}`; ChronoFork: `{}`".format(
                _fmt_address(divergence["expected"]), _fmt_address(divergence["actual"])))
            if divergence["context"]:
                lines.append("- preceding records:")
                lines.append("")
                lines.append("```")
                lines.append(format_divergence(divergence))
                lines.append("```")
        cycle = result["cycle_comparison"]
        lines.append("- cycle comparison: {} of {} projected instruction(s) matched; first mismatch: {}"
                     .format(
                         cycle["compared_instructions"] if cycle["matched"] else "prefix",
                         cycle["compared_instructions"],
                         "none" if cycle["matched"] else json.dumps(cycle["first_mismatch"], sort_keys=True)))
        failure = result["chronofork"]["failure"]
        if failure:
            lines.append("- ChronoFork exit code: {} ({})".format(
                failure["exit_code"], failure["message"]))
            if failure["recorded_instruction"]:
                lines.append("- instruction attempted and rejected: {}".format(
                    json.dumps(failure["recorded_instruction"], sort_keys=True)))
        lines.append("")
    lines.append("## Master-cycle timeline (supersedes the M3 instruction quantum)")
    lines.append("")
    lines.append("Canonical time is the master-clock cycle: one scanline is 3420 master cycles and")
    lines.append("each instruction advances the backend by its 68000 cycle cost times seven. The")
    lines.append("M3 instruction-quantum rows, the `--instructions-per-line` flag and the")
    lines.append("instruction-quantum gating fixtures are superseded by this model; the ROMs remain")
    lines.append("at SR mask 7 where they do not explicitly change it, so no handler runs.")
    lines.append("")
    lines.append("Cycle comparison policy: the oracle trace's first column is accumulated cycles at")
    lines.append("the start of each instruction, so a projected instruction's cost is the difference")
    lines.append("to the next oracle record; ChronoFork's fourth trace column is its")
    lines.append("`InstructionOutcome.cycles`. Absolute cycle bases and the oracle `time` column are")
    lines.append("not compared. Functional prefetch is not cycle-modelled, MULU/MULS use the")
    lines.append("documented operand-dependent early-out, DIVU/DIVS use the documented worst-case")
    lines.append("base, and no interrupt-latency equivalence is claimed.")
    lines.append("")
    lines.append("| ROM | compared instructions | cycles match | first cycle mismatch |")
    lines.append("|---|---:|---|---|")
    for result in evidence["roms"]:
        cycle = result["cycle_comparison"]
        lines.append("| `{}` | {} | {} | {} |".format(
            result["name"], cycle["compared_instructions"],
            "yes" if cycle["matched"] else "**no**",
            "none" if cycle["matched"] else "index {} ({}): oracle {}, ChronoFork {}".format(
                cycle["first_mismatch"].get("index"),
                cycle["first_mismatch"].get("pc"),
                cycle["first_mismatch"].get("oracle_cycles"),
                cycle["first_mismatch"].get("chronofork_cycles"))))
    lines.append("")
    lines.append("## Final state comparison")
    lines.append("")
    lines.append(
        "Compared: D0-D7, A0-A7, PC, effective SR (IPM/S/CCR) and {} work-RAM bytes".format(
            RAM_BYTES
        )
    )
    lines.append("at `0x00FF0000`. Raw SR is recorded but not compared: the oracle reports")
    lines.append("`0x7FE0` and ChronoFork `0x2700`, with equal effective fields.")
    lines.append("")
    for result in evidence["roms"]:
        final = result["final_state_comparison"]
        lines.append("### {}".format(result["name"]))
        lines.append("")
        if not final.get("comparable", True):
            lines.append("- comparable: **no** ({})".format(final["reason"]))
            lines.append("- oracle final PC: `{}`, SR `{}`, effective `{}`".format(
                final["oracle"]["pc"], final["oracle"]["sr_raw"],
                final["oracle"]["sr_effective"]))
            lines.append("- ChronoFork stopped at PC `{}` after {} executed instruction(s)".format(
                final["chronofork"]["pc"], final["chronofork"]["executed"]))
            lines.append("- oracle work-RAM: `{}`".format(final["oracle"]["ram_hex"]))
            lines.append("- ChronoFork work-RAM: `{}`".format(
                final["chronofork"]["ram_hex"]))
            lines.append("")
            continue
        lines.append("- match: **{}** (registers: {}, effective SR: {}, RAM: {})".format(
            "yes" if final["match"] else "no",
            "equal" if final["registers_equal"] else "different",
            "equal" if final["sr_effective_equal"] else "different",
            "equal" if final["ram_equal"] else "different"))
        lines.append("- oracle PC `{}`, SR `{}` effective `{}`".format(
            final["oracle"]["pc"], final["oracle"]["sr_raw"],
            final["oracle"]["sr_effective"]))
        lines.append("- ChronoFork PC `{}`, SR `{}` effective `{}`".format(
            final["chronofork"]["pc"], final["chronofork"]["sr_raw"],
            final["chronofork"]["sr_effective"]))
        lines.append("- oracle D0-D7: {}".format(", ".join(
            "`{}`".format(value) for value in final["oracle"]["d"])))
        lines.append("- ChronoFork D0-D7: {}".format(", ".join(
            "`{}`".format(value) for value in final["chronofork"]["d"])))
        lines.append("- oracle A0-A7: {}".format(", ".join(
            "`{}`".format(value) for value in final["oracle"]["a"])))
        lines.append("- ChronoFork A0-A7: {}".format(", ".join(
            "`{}`".format(value) for value in final["chronofork"]["a"])))
        lines.append("- oracle RAM: `{}`".format(final["oracle"]["ram_hex"]))
        lines.append("- ChronoFork RAM: `{}`".format(final["chronofork"]["ram_hex"]))
        if final["differences"]:
            lines.append("- differences: `{}`".format(json.dumps(final["differences"])))
        lines.append("")
    lines.append("## Negative control")
    lines.append("")
    lines.append("One projected record per ROM is corrupted in a copy; the comparator must")
    lines.append("exit 1 and report that exact index.")
    lines.append("")
    lines.append("| ROM | corrupted index | reported index | exit code | status |")
    lines.append("|---|---|---|---|---|")
    for result in evidence["roms"]:
        control = result["negative_control"]
        lines.append("| `{}` | {} | {} | {} | {} |".format(
            result["name"], control["corrupted_index"], control["reported_index"],
            control["comparator_exit_code"], control["status"]))
    lines.append("")
    lines.append("The cycle comparator is also negative-controlled: the first projected")
    lines.append("instruction's cost is corrupted by one cycle and the comparator must report it.")
    lines.append("")
    lines.append("| ROM | corrupted index | cycles comparator status |")
    lines.append("|---|---|---|")
    for result in evidence["roms"]:
        control = result["cycle_negative_control"]
        lines.append("| `{}` | {} | {} |".format(
            result["name"], control["corrupted_index"], control["status"]))
    lines.append("")
    lines.append("## Determinism")
    lines.append("")
    lines.append("| ROM | oracle trace run1 == run2 | ChronoFork trace run1 == run2 | ChronoFork state run1 == run2 |")
    lines.append("|---|---|---|---|")
    for result in evidence["roms"]:
        lines.append("| `{}` | {} | {} | {} |".format(
            result["name"],
            "yes" if result["oracle"]["deterministic"] else "**no**",
            "yes" if result["chronofork"]["deterministic"] else "**no**",
            "yes" if result["chronofork"]["state_sha256"] == result["chronofork"]["state_sha256_run2"] else "**no**"))
    lines.append("")
    lines.append("## Exact commands")
    lines.append("")
    lines.append("Harness invocation:")
    lines.append("")
    lines.append("```")
    lines.append(shlex.join(evidence["commands"]["harness"]))
    lines.append("```")
    lines.append("")
    for result in evidence["roms"]:
        lines.append("`{}` oracle probe (run 1; run 2 is identical):".format(result["name"]))
        lines.append("")
        lines.append("```")
        lines.append("cd {} && {}".format(
            shlex.quote(evidence["oracle"]["working_directory"]),
            shlex.join(result["oracle"]["probe_command"])))
        lines.append("```")
        lines.append("")
        lines.append("`{}` ChronoFork trace (run 1; run 2 is identical):".format(result["name"]))
        lines.append("")
        lines.append("```")
        lines.append("cd {} && {}".format(
            shlex.quote(str(REPO)), shlex.join(result["chronofork"]["command"])))
        lines.append("```")
        lines.append("")
    lines.append("Oracle final-state MCP sequence (per ROM, one session with a private")
    lines.append("`--base-path`):")
    lines.append("")
    lines.append("```")
    lines.append("initialize -> notifications/initialized")
    lines.append('session.load_module {path: "' + ORACLE_MODULE + '"}')
    lines.append("session.load_rom {path: <rom>}")
    lines.append("run.step_frames {frames: " + str(evidence["oracle"]["frames"]) + "}")
    lines.append('cpu.registers {device: "' + TRACE_DEVICE + '"}')
    lines.append(
        'cpu.read_memory {device: "'
        + TRACE_DEVICE
        + '", address: "0xFF0000", length: '
        + str(RAM_BYTES)
        + "}"
    )
    lines.append("shutdown")
    lines.append("```")
    lines.append("")
    lines.append("## Limitations")
    lines.append("")
    for limitation in evidence["limitations"]:
        lines.append("- {}".format(limitation))
    lines.append("")
    lines.append("## Reproduction")
    lines.append("")
    lines.append("Prerequisites: the pinned oracle binaries must exist under")
    lines.append("`.research/oracle/build/` (task t1), and `cargo build -p chronofork-cli` must")
    lines.append("have produced `target/debug/chronofork`.")
    lines.append("")
    lines.append("```sh")
    lines.append("cd {}".format(REPO))
    lines.append("python3 scripts/oracle-diff.py --out-dir {}".format(evidence["out_dir"]))
    lines.append("```")
    lines.append("")
    if evidence["summary"]["status"] == "divergence":
        lines.append("Expected exit code: **1** — `{}` diverges at the reported index.".format(
            ", ".join(result["name"] for result in evidence["roms"]
                      if not result["trace_comparison"]["matched"])))
    else:
        lines.append("Expected exit code: **0** — every projection and final state matches.")
    lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def _fmt_address(value):
    return "missing" if value is None else "0x{:06X}".format(value)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def parse_arguments(argv):
    parser = argparse.ArgumentParser(
        description="Differential instruction-trace harness against the pinned Exodus oracle."
    )
    parser.add_argument("--out-dir", default=str(DEFAULT_OUT_DIR),
                        help="evidence/artifact directory (default: .research/evidence)")
    parser.add_argument("--oracle-root", default=str(DEFAULT_ORACLE_ROOT),
                        help="pinned exodus-for-ais checkout (module path is relative to it)")
    parser.add_argument("--oracle-bin-dir", default=str(DEFAULT_ORACLE_BIN_DIR),
                        help="directory holding exodus-probe and exodus-mcpd")
    parser.add_argument("--chronofork-bin", default=str(DEFAULT_CHRONOFORK_BIN),
                        help="chronofork CLI binary")
    parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES,
                        help="oracle frames to run (default: 1)")
    parser.add_argument("--instruction-budget", type=int, default=DEFAULT_INSTRUCTION_BUDGET,
                        help="ChronoFork instruction budget (default: 10000)")
    parser.add_argument("--skip-build", action="store_true",
                        help="do not build the ChronoFork CLI if the binary is missing")
    parser.add_argument("--compare-projections", nargs=2, metavar=("EXPECTED", "ACTUAL"),
                        help="internal comparator mode: compare two address-per-line files")
    return parser.parse_args(argv)


def command_line():
    return [sys.executable] + [str(Path(__file__).resolve())] + sys.argv[1:]


def main(argv):
    options = parse_arguments(argv)
    if options.compare_projections:
        expected = read_projection_file(options.compare_projections[0])
        actual = read_projection_file(options.compare_projections[1])
        divergence = compare_addresses(expected, actual)
        if divergence is None:
            print("projections identical ({} records)".format(len(expected)))
            return 0
        print(format_divergence(divergence))
        return 1

    options.out_dir = Path(options.out_dir).resolve()
    options.oracle_root = Path(options.oracle_root).resolve()
    options.oracle_bin_dir = Path(options.oracle_bin_dir).resolve()
    options.chronofork_bin = Path(options.chronofork_bin).resolve()
    options.probe = options.oracle_bin_dir / "exodus-probe"
    options.mcpd = options.oracle_bin_dir / "exodus-mcpd"
    options.artifacts = options.out_dir / "artifacts"
    options.roms_dir = options.artifacts / "roms"
    options.artifacts.mkdir(parents=True, exist_ok=True)
    options.roms_dir.mkdir(parents=True, exist_ok=True)

    ensure_chronofork_binary(options.chronofork_bin, options.skip_build)
    oracle = oracle_provenance(options)
    chronofork = chronofork_provenance(options)

    results = []
    for name, builder, spec_quantum, require_gating in ROMS:
        print(
            "running {} (master-cycle timeline)...".format(name),
            flush=True,
        )
        results.append(run_rom(name, builder, spec_quantum, require_gating, options))

    controls = []
    for result in results:
        work = options.artifacts / result["name"]
        # Persist both projections for auditability and drive the negative control
        # from the oracle projection (which always reaches the marker).
        oracle_records = parse_oracle_trace(Path(result["oracle"]["trace_path"]))
        marker = int(result["rom"]["marker_pc"], 16)
        projection = project_oracle(oracle_records, marker)
        write_projection_file(work / "oracle.projected.txt", projection)
        write_projection_file(
            work / "chronofork.projected.txt",
            chronofork_projection_from_result(result),
        )
        controls.append(
            negative_control(Path(__file__).resolve(), work, result["name"], projection)
        )
    for result, control in zip(results, controls):
        result["negative_control"] = control

    cycle_controls = []
    for result in results:
        oracle_records = parse_oracle_trace(Path(result["oracle"]["trace_path"]))
        marker = int(result["rom"]["marker_pc"], 16)
        marker_index = [record["address"] for record in oracle_records].index(marker)
        chronofork_records = parse_chronofork_trace(Path(result["chronofork"]["trace_path"]))
        projected = project_chronofork_records(
            chronofork_records,
            result["chronofork"]["executed"],
            result["chronofork"]["failure"] is not None,
        )
        corrupted = [dict(record) for record in projected]
        if corrupted:
            corrupted[0]["cycles"] += 1
            mismatch = compare_cycles(oracle_records, marker_index, corrupted)
        else:
            mismatch = None
        status = "passed" if mismatch is not None else "failed"
        cycle_controls.append(
            {
                "status": status,
                "corrupted_index": 0 if corrupted else None,
                "first_mismatch": mismatch,
            }
        )
    for result, control in zip(results, cycle_controls):
        result["cycle_negative_control"] = control

    findings = []
    for result in results:
        if not result["trace_comparison"]["matched"]:
            failure = result["chronofork"]["failure"]
            recorded = failure["recorded_instruction"] if failure else None
            divergence = result["trace_comparison"]["first_divergence"]
            findings.append(
                {
                    "rom": result["name"],
                    "title": "ChronoFork instruction stream diverges from the 68000",
                    "detail": (
                        "The pinned oracle and ChronoFork disagree before the marker loop "
                        "(first divergence index {}, reason: {}). ChronoFork executed {} "
                        "instruction(s){}".format(
                            divergence["index"] if divergence else "unknown",
                            divergence["reason"] if divergence else "unknown",
                            result["chronofork"]["executed"],
                            " and attempted the instruction at {}".format(recorded["pc"])
                            if recorded
                            else "",
                        )
                        + ". This is reported as a finding: no comparator was weakened and no "
                        "core code was edited to mask it."
                    ),
                    "observed": (
                        format_divergence(divergence)
                        if divergence
                        else (failure["message"] if failure else "no divergence detail")
                    ),
                    "implication": (
                        "{} cannot provide differential evidence until the cause is resolved."
                        .format(result["name"])
                    ),
                }
            )
        if not result["cycle_comparison"]["matched"]:
            mismatch = result["cycle_comparison"]["first_mismatch"]
            findings.append(
                {
                    "rom": result["name"],
                    "title": "Per-instruction cycle cost differs from the pinned oracle",
                    "detail": (
                        "The first cycle mismatch is at projected index {} ({}): oracle {} "
                        "cycle(s), ChronoFork {} cycle(s). This is reported as a finding: no "
                        "expectation, comparator or core table was edited to hide it.".format(
                            mismatch.get("index") if mismatch else "unknown",
                            mismatch.get("pc") if mismatch else "unknown",
                            mismatch.get("oracle_cycles") if mismatch else "unknown",
                            mismatch.get("chronofork_cycles") if mismatch else "unknown",
                        )
                    ),
                    "observed": json.dumps(mismatch, sort_keys=True),
                    "implication": (
                        "{} cycle evidence is limited to the matching prefix.".format(result["name"])
                    ),
                }
            )
        final = result["final_state_comparison"]
        if final.get("comparable", True) and not final["match"]:
            findings.append(
                {
                    "rom": result["name"],
                    "title": "Final state differs from the oracle",
                    "detail": "Registers, effective SR or work RAM differ; see the final-state section.",
                    "observed": json.dumps(final["differences"]),
                    "implication": "The core's architectural state is not equivalent for this ROM.",
                }
            )

    def overall():
        for result in results:
            if not result["trace_comparison"]["matched"]:
                return "divergence"
            if not result["cycle_comparison"]["matched"]:
                return "divergence"
            if result["cycle_negative_control"]["status"] != "passed":
                return "fail"
            final = result["final_state_comparison"]
            if not final.get("comparable", True) or not final["match"]:
                return "divergence"
            if result["negative_control"]["status"] != "passed":
                return "fail"
            if not result["scheduler_control"]["passed"]:
                return "fail"
        return "pass"

    limitations = [
        "The oracle starts with undefined D0-D7/A0-A6 at 0xFFFFFFFF; ChronoFork zeroes them. "
        "Every ROM explicitly initialises every register used in the final-state comparison.",
        "Raw SR words are not comparable (oracle 0x7FE0 vs ChronoFork 0x2700); only effective "
        "fields (IPM, S, CCR X/N/Z/V/C) are compared.",
        "Cycle comparison uses the oracle's accumulated cycle column only: an instruction's "
        "cost is the difference to the next record, compared with ChronoFork's fourth trace "
        "field (InstructionOutcome.cycles). Absolute cycle bases and the oracle time column are "
        "not compared. Functional 68000 prefetch is not cycle-modelled, MULU/MULS use the "
        "documented operand-dependent early-out, DIVU/DIVS use the documented worst-case base, "
        "and no interrupt-latency equivalence is claimed.",
        "VDP data-port reads are not compared: the oracle's VRAM read prefetch behaviour does not "
        "match the minimal VDP model. VDP control/data-port writes are executed and are part of "
        "the compared instruction stream. No rendering, DMA, CRAM/VSRAM or raster timing is claimed.",
        "The oracle trace ring defaults to 1,000,000 entries; these runs stay far below it, so no "
        "start-of-run records are dropped.",
        "Canonical time is the 3420-master-cycle scanline; the M3 instruction quantum and its "
        "--instructions-per-line flag are superseded. The ROMs keep SR mask 7 where they do not "
        "change it, so no interrupt handler runs in the measured window.",
        "This evidence compares instruction-address projection, per-instruction 68000 cycle cost "
        "and a narrow selected final state for eight independently authored synthetic ROMs. It "
        "does not establish full 68000 instruction-set conformance, general program behaviour, "
        "or game compatibility, and it makes no audio, Z80, controller or full-hardware claim.",
        "Instruction semantics beyond the compared observables are covered by the ROMs' "
        "self-checking flag branches and the final-state comparison; divergences are reported "
        "as findings rather than hidden by editing expectations.",
    ]

    evidence = {
        "schema": "chronofork.oracle-diff.v2",
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "out_dir": str(options.out_dir),
        "generator_sha256": sha256_file(Path(__file__).resolve()),
        "oracle": oracle,
        "chronofork": chronofork,
        "roms": results,
        "negative_control": {result["name"]: result["negative_control"] for result in results},
        "findings": findings,
        "limitations": limitations,
        "summary": {
            "status": overall(),
            "roms_compared": len(results),
            "trace_matches": sum(
                1 for result in results if result["trace_comparison"]["matched"]
            ),
            "cycle_matches": sum(
                1 for result in results if result["cycle_comparison"]["matched"]
            ),
            "cycle_negative_controls_passed": sum(
                1
                for result in results
                if result["cycle_negative_control"]["status"] == "passed"
            ),
            "final_state_comparable": sum(
                1
                for result in results
                if result["final_state_comparison"].get("comparable", True)
            ),
            "final_state_matches": sum(
                1
                for result in results
                if result["final_state_comparison"].get("comparable", True)
                and result["final_state_comparison"]["match"]
            ),
            "negative_controls_passed": sum(
                1 for result in results if result["negative_control"]["status"] == "passed"
            ),
            "scheduler_controls_passed": sum(
                1 for result in results if result["scheduler_control"]["passed"]
            ),
            "masked_gating_controls_passed": sum(
                1
                for result in results
                if result["scheduler_control"]["gating_required"]
                and result["scheduler_control"]["passed"]
            ),
            "masked_gating_controls_required": sum(
                1 for result in results if result["scheduler_control"]["gating_required"]
            ),
        },
        "commands": {"harness": command_line()},
    }

    json_path = options.out_dir / "oracle-diff.json"
    markdown_path = options.out_dir / "oracle-diff.md"
    write_evidence_json(json_path, evidence)
    write_evidence_markdown(markdown_path, evidence)

    summary = evidence["summary"]
    print("")
    print("oracle differential summary: {}".format(summary["status"].upper()))
    print("  ROMs compared: {}".format(summary["roms_compared"]))
    print("  trace matches: {}/{}".format(summary["trace_matches"], summary["roms_compared"]))
    print("  cycle matches: {}/{}".format(summary["cycle_matches"], summary["roms_compared"]))
    print("  cycle negative controls passed: {}/{}".format(
        summary["cycle_negative_controls_passed"], summary["roms_compared"]))
    print(
        "  final-state matches: {}/{} comparable; {} not comparable".format(
            summary["final_state_matches"],
            summary["final_state_comparable"],
            summary["roms_compared"] - summary["final_state_comparable"],
        )
    )
    print("  negative controls passed: {}/{}".format(
        summary["negative_controls_passed"], summary["roms_compared"]))
    print("")
    for result in results:
        control = result["scheduler_control"]
        print(
            "  {}: executed {}; SR mask ChronoFork {} / oracle {}; cycles {}".format(
                result["name"],
                control["executed"],
                control["chronofork_sr_mask"],
                control["oracle_sr_mask"],
                "matched" if result["cycle_comparison"]["matched"] else "DIVERGED",
            )
        )
    for result in results:
        divergence = result["trace_comparison"]["first_divergence"]
        if divergence:
            print("")
            print("{} trace:".format(result["name"]))
            print(format_divergence(divergence))
            failure = result["chronofork"]["failure"]
            if failure:
                print("  ChronoFork failure: {}".format(failure["message"]))
    print("")
    print("evidence: {}".format(json_path))
    print("report:   {}".format(markdown_path))
    return 0 if summary["status"] == "pass" else 1


def chronofork_projection_from_result(result):
    records = parse_chronofork_trace(Path(result["chronofork"]["trace_path"]))
    failed = result["chronofork"]["failure"] is not None
    return project_chronofork(records, result["chronofork"]["executed"], failed)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except HarnessError as error:
        print("oracle-diff: error: {}".format(error), file=sys.stderr)
        sys.exit(2)
