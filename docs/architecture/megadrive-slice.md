# Mega Drive 68000 Slice

- **Status:** Partial real-hardware slice implemented (68000 + ROM/RAM + VDP port
  window + interrupt scheduler); not a complete emulator.
- **Crate:** `crates/chronofork-megadrive`
- **Backend ID:** `chronofork.megadrive.m68000`
- **Related:** [ADR-0002](../adr/0002-backend-boundaries.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)

## Purpose

This slice starts ChronoFork's Rust-native production backend under the accepted
staged-hybrid strategy. It proves that a real 68000 reset/execute/VDP/interrupt/
snapshot/fork path can sit behind the same backend contract as the synthetic
validator, without pretending the result is a playable Mega Drive emulator.

No upstream emulator source code has been incorporated. The program and all test
fixtures are independently authored synthetic data.

## Implemented

### Memory bus

| Region | Address | Behavior |
|---|---|---|
| Cartridge ROM | `0x000000..0x3FFFFF` | Immutable, big-endian; only the provided image length is valid |
| Work RAM | `0xFF0000..0xFFFFFF` | 64 KiB, big-endian, mirrored by address bits 15–0 |
| VDP ports | `0xC00000..0xC0000F` | Data port `0x00..0x03`, control/status port `0x04..0x07`, HV counter `0x08..0x0F` |
| Everything else | — | Explicit `UnmappedRead` / `UnmappedWrite` error |

- Word and long accesses require even addresses.
- ROM writes are rejected.
- Multi-byte and debugger writes validate the entire range before mutating RAM, so
  a failed write cannot leave partial state.
- PSG (`0xC00011` and above), Z80 (`0xA00000`) and controller (`0xA10000`) windows
  are deliberately unmapped and return explicit errors; they are not open bus.
- VDP mirrors beyond `0xC0000F` and the physical byte-swapped/interleaved VRAM
  layout are not modelled.
- Bus reads take `&mut self` because a VDP port read can advance the VDP address.
  Debugger reads (`read_memory`) are side-effect free: they use `peek8`, which
  rejects the side-effecting VDP window instead of changing device state.

### 68000 subset

Implemented:

| Instruction | Forms |
|---|---|
| `MOVEQ` | `#imm,Dn` |
| `MOVE` | `B/W/L` between `Dn`, `#imm` and absolute-long operands |
| `ADD` / `SUB` | `B/W/L` between data registers |
| `ADDQ` / `SUBQ` | word/long, data-register destination |
| `CMP` | `B/W/L`, data-register operands; `X` preserved |
| `TST` | `B/W/L`, data-register operand |
| `CLR` | `W/L`, data-register destination |
| `LEA` | `(abs).L,An` |
| `PEA` | `(abs).L` |
| `NOP` | — |
| `BRA`, `BSR`, `BEQ`, `BNE`, `BCC`, `BCS`, `BMI`, `BPL` | 8-bit displacement |
| `DBRA` | `Dn,disp16` |
| `JMP`, `JSR` | absolute-long operand |
| `RTS` | — |
| Interrupt exception entry | `M68000::interrupt(vector_address)` |

Reset loads the supervisor stack pointer from address `0x000000` and the program
counter from `0x000004`; `SR = 0x2700`, all general registers clear.

Flags implemented: `N`, `Z`, `V`, `C`, `X` for the listed arithmetic/logic
operations. `SR` upper bits are preserved.

The interrupt entry API pushes a word `SR` and a long return `PC` (six bytes),
sets the supervisor bit, reads the target long from the supplied vector-table
address and jumps to it. It wakes a halted processor. The caller owns priority
masking and vector selection. `RTE`, `STOP`, `ORI`/`ANDI` to `SR` and the rest of
the exception groups are not implemented.

### VDP status/control/VRAM slice

- 64 KiB flat VRAM, zeroed at reset, big-endian.
- Data port (`0xC00000..=0xC00003`), control/status port
  (`0xC00004..=0xC00007`) and HV-counter port (`0xC00008..=0xC0000F`).
- Two-word command setup (command code and 14+2 address bits) and register writes
  `0x8000 | register << 8 | value` for registers `0x00..=0x17`.
- Data-port VRAM reads/writes with auto-increment from register `0x0F`, wrapping
  at 64 KiB. Byte accesses transfer a full word; byte writes duplicate into both
  halves; byte reads select a lane.
- Status register with VBlank (driven by the scheduler's line position) and
  FIFO-empty (always set because no FIFO is modelled). All other status bits are
  documented constants; VINT-pending (bit 7) is not wired yet.
- Rejected targets are typed errors: DMA, CRAM, VSRAM, half-completed commands,
  out-of-range registers, HV-counter writes and offsets above `0x0F`.
- Versioned, fixed-length VDP snapshot (`CFVDP001`, 65 574 bytes) with atomic
  validated restore.

Not implemented: rendering, planes, window, sprites, scrolling, CRAM, VSRAM, DMA,
FIFO/access timing and H/V counters.

### Deterministic interrupt scheduler

- Instruction-quantum model: one scheduler tick equals one completed 68000
  instruction, not one CPU cycle. This is explicitly **not** cycle-accurate
  hardware timing.
- NTSC constants (262 lines/frame, 224 active) and PAL constants (313/240), with a
  configurable instruction quantum per scanline (default 64).
- HBlank is requested when each active scanline completes; VBlank when vertical
  blanking begins. Blanking lines are quiet.
- At most one request per level is retained; repeated requests coalesce with
  diagnostic counters. Delivery order is VBlank (level 6) before HBlank (level 4).
- The scheduler does not apply the CPU mask itself; `next_deliverable` exposes the
  68000 rule for callers.
- Versioned snapshot (`CFINTSCH`, 56 bytes plus pending tick fields) with atomic
  validated restore and configuration-mismatch rejection.

### Backend integration

- `MegadriveFactory` validates ROM size (8 bytes .. 4 MiB), hashes the immutable
  ROM and the scheduler configuration into `config_hash` (`CFMDCONF2`).
- `MegadriveBackend` implements the existing `Backend` contract: execution,
  snapshots, restore, inspection, memory reads/writes.
- Snapshot payload magic is `CFMD68K2`, payload version **2**, followed by the
  68000 register file, `PC`, `SR`, halted flag, tick, 64 KiB RAM, the complete VDP
  snapshot and the complete scheduler snapshot. Version 1 payloads are rejected
  rather than reinterpreted.
- The state hash (`CFMDSTATE2`) covers the 68000 state, tick, RAM, VDP snapshot and
  scheduler snapshot, so pending interrupts and VRAM changes are part of
  checkpoint identity.
- Restore validates backend identity, backend version, envelope version, config
  hash, payload magic/version/length, the VDP and scheduler snapshots, and the
  recomputed state hash. The replacement bus and scheduler are built before they
  are committed, so a rejected restore leaves the machine untouched.
- At most one interrupt is delivered at each instruction boundary, before the next
  instruction executes. A request raised by the scheduler at the end of a step is
  therefore inspectable at that tick and is taken deterministically on the next
  step. VBlank is taken before HBlank.
- VDP VBlank status is synchronized from the scheduler's scanline position.
- `MachineInspection::registers` additionally exposes `vdp_status`,
  `vdp_auto_increment`, `vdp_address`, `irq_mask`, `irq_pending_level`,
  `irq_vector`, `scheduler_frame` and `scheduler_line` as pseudo-registers. The
  schema has no dedicated device-inspection section yet; this keeps the generic
  backend API unchanged.
- Capabilities truthfully report no breakpoints, watchpoints, tracing, rendering or
  audio.

### Known limitations of the integration

- The 68000 `SR` interrupt mask is exposed but does not gate delivery: no
  instruction in the subset can change the mask, and the CPU entry API leaves
  masking to the caller. The reset mask (`7`) would otherwise suppress every
  request forever.
- Interrupt handlers cannot return correctly: `RTE` is not implemented. The demo
  handlers loop, and each delivery pushes six bytes, so a very long run
  eventually exhausts the supervisor stack and produces a typed bus error instead
  of corrupting state.
- Interrupt grouping is VBlank/HBlank only; Z80 interrupts, timer interrupts, DMA
  stalls and other 68000 exception groups are absent.
- The VDP VINT-pending status bit and controller-port interrupt acknowledge are not
  wired.

### Tests

`chronofork-megadrive` currently contributes **71 tests**:

- reset vectors and register initialization;
- every implemented 68000 instruction form, flags, `DBRA` termination and
  interrupt-entry success/error paths;
- ROM/RAM big-endian access, alignment, unmapped and atomic-write errors;
- VDP port mapping through the bus, register/VRAM round-trips, byte-lane behavior,
  side-effect-free debugger reads and explicit PSG/Z80/controller errors;
- VDP snapshot round-trip and malformed-input rejection;
- deterministic tick-driven HBlank/VBlank generation, coalescing, priority,
  scanline timing and scheduler snapshot rejection;
- VDP register/VRAM programming by the synthetic 68000 program through the bus;
- interrupt scheduling over ticks with CPU delivery, priority and pending
  inspection;
- snapshot/restore including VDP and pending-interrupt state, old/malformed payload
  rejection and configuration mismatch;
- session fork/restore with VDP and pending-interrupt state and source isolation;
- device-level determinism across two independent machines.

The workspace total is **98 tests** after this slice.

## Not implemented

- VDP rendering, planes, sprites, scrolling, CRAM/VSRAM, DMA and raster timing.
- YM2612, PSG or audio.
- Z80, bus arbitration, timer/other interrupts, `RTE`, and SR mask gating.
- Controller input or I/O ports.
- Cartridge mapper/SSF2, SRAM, region/TMSS behavior.
- 68000 prefetch, bus-cycle timing, separate user/supervisor stacks, most
  addressing modes and most instructions.
- Cycle accuracy. One executed instruction equals one ChronoFork tick in this
  slice; the scheduler maps instruction quanta to scanlines with configurable
  constants. This is deterministic but not hardware timing.
- ROM header validation, checksum, region detection or ROM database.
- Loading a user ROM through the CLI. The demo uses an independently authored
  synthetic ROM.

Unsupported opcodes and port accesses return errors; they are never silently
skipped.

## CLI demo

```sh
cargo run -p chronofork-cli -- megadrive demo
```

The demo executes `1 + 2`, programs VDP register `0x0F` (auto-increment 2), writes
and reads back VRAM word `0x1234`, captures the VDP status, shows an HBlank request
pending at tick 64 and delivered on the next step, shows a VBlank request at the
start of vertical blanking with a shorter demonstration quantum, restores a
pending-VBlank checkpoint, then forks, mutates and restores the session. It prints
`chronofork megadrive demo: PASS`.

## Next increments

1. Wire the VDP VINT-pending status bit and controller-port interrupt acknowledge.
2. Add `RTE` and SR mask handling so handlers can return and masking can gate
   delivery.
3. ROM header/region/config identity and a cartridge loader.
4. More 68000 addressing modes and instructions, driven by differential tests.
5. Exodus external-oracle traces for generated programs once Ninja/clang/libpng are
   available.
6. Controller input schema (multi-port) before claiming input support.
7. Rendering/DMA only after the non-rendering state model is stable.
