# Mega Drive 68000 Slice

- **Status:** First real-hardware slice implemented; not a complete emulator.
- **Crate:** `crates/chronofork-megadrive`
- **Backend ID:** `chronofork.megadrive.m68000`
- **Related:** [ADR-0002](../adr/0002-backend-boundaries.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)

## Purpose

This slice starts ChronoFork's Rust-native production backend under the accepted
staged-hybrid strategy. It proves that a real 68000 reset/execute/snapshot/fork
path can sit behind the same backend contract as the synthetic validator, without
pretending the result is a playable Mega Drive emulator.

## Implemented

### Memory bus

| Region | Address | Behavior |
|---|---|---|
| Cartridge ROM | `0x000000..0x3FFFFF` | Immutable, big-endian; only the provided image length is valid |
| Work RAM | `0xFF0000..0xFFFFFF` | 64 KiB, big-endian, mirrored by address bits 15–0 |
| Everything else | — | Explicit `UnmappedRead` / `UnmappedWrite` error |

- Word and long accesses require even addresses.
- ROM writes are rejected.
- Multi-byte and debugger writes validate the entire range before mutating RAM, so
  a failed write cannot leave partial state.

### 68000 subset

Implemented:

| Instruction | Forms |
|---|---|
| `MOVEQ` | `#imm,Dn` |
| `MOVE` | `B/W/L` between `Dn`, `#imm` and absolute-long operands |
| `ADD` / `SUB` | `B/W/L` between data registers |
| `ADDQ` / `SUBQ` | word/long, data-register destination |
| `CLR` | `W/L`, data-register destination |
| `NOP` | — |
| `BRA`, `BSR`, `BEQ`, `BNE` | 8-bit displacement |
| `JMP`, `JSR` | absolute-long operand |
| `RTS` | — |

Reset loads supervisor stack pointer from address `0x000000` and program counter
from `0x000004`; `SR = 0x2700`, all general registers clear.

Flags implemented: `N`, `Z`, `V`, `C`, `X` for the listed arithmetic/logic
operations. `SR` upper bits are preserved.

### Backend integration

- `MegadriveFactory` validates ROM size (8 bytes .. 4 MiB) and hashes the immutable
  ROM into `config_hash`.
- `MegadriveBackend` implements the existing `Backend` contract:
  execution, snapshots, restore, inspection, memory reads/writes.
- Snapshot payload has magic `CFMD68K1`, version 1, full register file, `PC`, `SR`,
  halted flag, tick and 64 KiB RAM.
- State hash covers registers, `PC`, `SR`, halted flag, tick and RAM.
- Restore validates backend identity, version, payload version, configuration hash
  and recomputed state hash before applying anything.
- Capabilities truthfully report no breakpoints, watchpoints, tracing, rendering or
  audio.

### Tests

`crates/chronofork-megadrive` currently contributes 13 tests:

- reset vectors and register initialization;
- `MOVEQ` + `ADD` execution;
- ROM/RAM big-endian access, alignment and unmapped errors;
- atomic rejection of invalid multi-byte writes;
- snapshot round-trip and configuration-mismatch rejection;
- deterministic convergence of two independent machines;
- session fork/restore/isolation on the real core;
- illegal opcode and invalid ROM errors;
- debugger write protection for ROM.

The workspace total is **40 tests** after this slice.

## Not implemented

- VDP, VRAM/CRAM, DMA, rendering or video timing.
- YM2612, PSG or audio.
- Z80, bus arbitration, interrupts or exception processing.
- Controller input or I/O ports.
- Cartridge mapper/SSF2, SRAM, region/TMSS behavior.
- 68000 prefetch, bus-cycle timing, privilege separation, most addressing modes
  and most instructions.
- Cycle accuracy. One executed instruction equals one ChronoFork tick in this
  slice; this is a deliberate deterministic quantum, not hardware timing.
- ROM header validation, checksum, region detection or ROM database.
- Loading a user ROM through the CLI. The demo uses an independently authored
  synthetic ROM.

Unsupported opcodes return an error; they are never silently skipped.

## CLI demo

```sh
cargo run -p chronofork-cli -- megadrive demo
```

The demo executes `1 + 2`, stores the result at `0x00FF0000`, reloads it into `D2`,
snapshots, forks, mutates the fork's RAM, verifies isolation and restores the
checkpoint.

## Next increments

1. VDP status/control registers and a minimal VRAM window, with explicit
   capability flags.
2. Interrupt vectors and a deterministic interrupt/tick scheduler.
3. ROM header/region/config identity and a cartridge loader.
4. More 68000 addressing modes and instructions, driven by differential tests.
5. Exodus external-oracle traces for generated programs once Ninja/clang/libpng are
   available.
6. Controller input schema (multi-port) before claiming input support.
