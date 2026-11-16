# Mega Drive 68000 Slice

- **Status:** Partial real-hardware slice implemented (68000 + ROM/RAM + VDP port
  window + VBlank/HBlank scheduler with SR-mask gating, `RTE` and VDP
  VINT-pending/acknowledge + bounded debug instruction trace, with passing
  three-ROM oracle differential evidence including a default-quantum masked-ROM
  control); not a complete emulator. Controller-port data-register reads for real-hardware
  input TAS are implemented and independently verified for the TH-high 3-button subset on
  ports 1–2 ([ADR-0009](../adr/0009-real-hardware-input-tas.md); t5 PASS, 2026-11-16), with
  the canonical state, demos and M1 evidence unchanged. A/Start, TH/6-button/Start
  multiplexing, controller timing, analog/keyboard hardware and port 3 remain absent.
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
| Controller data registers | `0x00A10003`, `0x00A10005` | Byte reads only, driven by the current step's `InputFrame` ports 0/1; see below |
| Everything else | — | Explicit `UnmappedRead` / `UnmappedWrite` error |

- Word and long accesses require even addresses.
- ROM writes are rejected.
- Multi-byte and debugger writes validate the entire range before mutating RAM, so
  a failed write cannot leave partial state.
- PSG (`0xC00011` and above), Z80 (`0xA00000`) and the rest of the controller
  (`0xA10000..0xA1001F`) window are deliberately unmapped and return explicit errors; they
  are not open bus. Only byte reads of the two controller data registers are mapped, for
  execution only; all controller writes and debugger reads remain typed errors.
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
| `RTE` | `0x4E73`; restores raw `SR` and PC from the six-byte frame, advances A7 |
| `MOVE.W #imm,SR` | `0x46FC`; replaces raw `SR`, including the interrupt mask |
| Interrupt exception entry | `M68000::interrupt(level, vector_address)`: pushes raw `SR` + PC, sets `S`, preserves CCR, loads the accepted level into the IPM |

Absolute-long `MOVE` loads use the real 68000 source mode/register `111/001`
(`0x1039`/`0x2039`/`0x3039`) and fetch a 4-byte address. The absolute-*word* source
forms (`0x1038`/`0x2038`/`0x3038`) are not in this subset and return the typed
`IllegalOpcode` error instead of being executed with a 32-bit fetch. An earlier
decoder accepted the `0x..38` load forms; EPIC-0002 E2-13 repaired it and regression
tests cover byte/word/long `(abs).L,Dn` loads, rejection of the `0x..38` forms and
the `Dn,(abs).L` store encodings.

Reset loads the supervisor stack pointer from address `0x000000` and the program
counter from `0x000004`; `SR = 0x2700`, all general registers clear.

Flags implemented: `N`, `Z`, `V`, `C`, `X` for the listed arithmetic/logic
operations. `SR` upper bits are preserved.

The interrupt entry API pushes a word raw `SR` and a long return `PC` (six bytes),
sets the supervisor bit, preserves the CCR, loads the accepted interrupt level into
the interrupt-priority mask, reads the target long from the supplied vector-table
address and jumps to it. It wakes a halted processor. Invalid levels are rejected
before any state changes; stack or vector faults return typed errors (a failed first
push leaves `SR`/`PC` untouched, while the six-byte frame can already be pushed
before a later vector or target fault). `RTE` validates the complete frame before
changing canonical state, restores the exact raw `SR` (IPM, `S`, CCR), restores the
PC and advances A7 by six. `MOVE.W #imm,SR` (`0x46FC`) replaces the raw `SR` so a
program can change its mask. On real hardware `MOVE.W #imm,SR`, `RTE`, `STOP` and
other privileged operations require supervisor mode; **privilege is not enforced in
this slice**. `STOP`, `ORI`/`ANDI`/`EORI` to `SR` and the other exception groups are
not implemented.

### VDP status/control/VRAM slice

- 64 KiB flat VRAM, zeroed at reset, big-endian.
- Data port (`0xC00000..=0xC00003`), control/status port
  (`0xC00004..=0xC00007`) and HV-counter port (`0xC00008..=0xC0000F`).
- Two-word command setup (command code and 14+2 address bits) and register writes
  `0x8000 | register << 8 | value` for registers `0x00..=0x17`.
- Data-port VRAM reads/writes with auto-increment from register `0x0F`, wrapping
  at 64 KiB. Byte accesses transfer a full word; byte writes duplicate into both
  halves; byte reads select a lane.
- Status register with VBlank (bit 3, driven by the scheduler's line position),
  VINT-pending (bit 7, derived from the backend's pending VBlank request) and
  FIFO-empty (bit 8, always set because no FIFO is modelled). The VINT latch is set
  while a VBlank (level 6) request is pending — including while the `SR` mask blocks
  it — and clears when that request is delivered and acknowledged; a status-port
  read returns the current status and cancels only a half-latched command word, and
  never acknowledges, clears or consumes an interrupt. The latch is derived state,
  not serialized into the VDP snapshot; the backend recomputes it after every step
  and after restore. All other status bits are documented constants.
- VDP interrupt-enable registers and VDP-side acknowledge logic are not modelled:
  the status bit is driven by the external scheduler, and only delivery of the VDP
  source acknowledges it.
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
  diagnostic counters, and a masked request keeps its original issue tick. Delivery
  order is VBlank (level 6) before HBlank (level 4).
- The scheduler keeps pending requests independently of the CPU mask and exposes
  `next_deliverable(raw_sr)`, which applies the shared 68000 rule: level 7 is always
  deliverable and every other level must be strictly above the `SR` interrupt
  priority mask. The backend, not the scheduler, performs delivery and acknowledge.
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
  instruction executes, and only when `next_deliverable(cpu.sr())` admits it: level 7
  is always taken and every other level must be strictly above the IPM. A request
  raised by the scheduler at the end of a step is therefore inspectable at that tick
  and is taken deterministically on a later step if the mask admits it. VBlank
  (level 6) is taken before HBlank (level 4).
- Masked requests are retained with their original issue tick and are not
  acknowledged, dropped or duplicated. Delivery acknowledges only the level that
  entered the CPU, so a pending request at the other level survives. A failed
  exception entry acknowledges nothing. Exception entry loads the accepted level
  into the handler's IPM, so a lower-priority VDP request cannot nest before `RTE`
  restores the interrupted raw `SR`.
- VDP VBlank status is synchronized from the scheduler's scanline position. The
  derived VINT-pending status bit is recomputed from the scheduler's pending VBlank
  request after every step, after delivery and on restore; it is not part of the
  serialized VDP snapshot.
- `MachineInspection::registers` additionally exposes `vdp_status`,
  `vdp_auto_increment`, `vdp_address`, `irq_mask`, `irq_pending_level`,
  `irq_deliverable`, `irq_vector`, `scheduler_frame` and `scheduler_line` as
  pseudo-registers. `irq_deliverable` is 1 when the highest-priority pending
  request is admitted by the current SR mask and 0 while it is masked. The schema
  has no dedicated device-inspection section yet; this keeps the generic backend
  API unchanged.
- Capabilities truthfully report no breakpoints, watchpoints, tracing, rendering or
  audio. The bounded trace is a concrete-type debug facility, not the generic
  `Backend` `tracing` capability.

### Bounded instruction trace (debug only)

- `trace` module: `TraceEntry` carries the zero-based instruction `index`, fetched
  `pc` and `opcode` word; `InstructionTrace` is a fixed-capacity ring.
- Disabled by default. `M68000` and `MegadriveBackend` expose enable/disable/reset/
  trace/take; the generic `Backend` trait has no trace method.
- One record is appended after a successful opcode fetch and before decode, so an
  instruction that then fails as illegal or unsupported is still visible. A fetch
  that fails before the opcode word records nothing; interrupt exception entry is
  not an instruction and is not recorded.
- Capacity must be at least 1 (`TraceError::ZeroCapacity`). Indexes keep counting
  when the ring evicts its oldest records, and `dropped`/`first_index` expose loss;
  storage is allocated lazily and bounded by the capacity.
- Debug data only: enabling the trace leaves snapshot payload bytes, versions, state
  hashes and configuration hashes identical, and restore does not rewind the trace.
- CLI `megadrive trace` runs an instruction budget or stops before `--stop-pc`,
  writes TSV `index<TAB>pc_hex<TAB>opcode_hex` and a deterministic key=value final
  state (executed, tick, quantum, video, halted, D0–D7, A0–A7, PC, raw SR,
  RAM base/length/hex), and honours `--instructions-per-line`, `--video`,
  `--trace-capacity`, `--trace-out` (default stdout) and required `--state-out`.
  The default quantum of 64 is unchanged.

### Oracle differential method and limits

The reproducible harness `scripts/oracle-diff.py` compares the minimal core against
a pinned Exodus headless oracle for three independently authored synthetic ROMs; the
full numbers, commands and limitations are in
[Oracle differential instruction-trace evidence](../research/oracle-differential-evidence.md).

- Both sides produce an instruction trace; the harness projects each to the ordered
  sequence of executed instruction **addresses** before a marker self-loop. The
  oracle's `cycle` and `time` columns are ignored. Because the oracle renders
  opcodes as mnemonic+args while ChronoFork stores raw words, every ChronoFork
  projected opcode is checked against the ROM word at that address instead of being
  compared directly to the oracle column.
- It asserts equal projection length and addresses, reports record counts and
  projection SHA-256, reports the first divergence with preceding context, and exits
  1 on divergence. A per-ROM negative control corrupts one projected record and
  requires the comparator to fail at that index.
- Final state is compared through the oracle's MCP `cpu.registers` and
  `cpu.read_memory` calls: D0–D7, A0–A7, PC, effective SR fields (IPM/S/CCR) and 64
  work-RAM bytes at `0xFF0000`. Raw SR words differ and are not compared.
- The harness records the per-ROM scheduler quantum read back from the state file
  and, for `rom_masked`, checks the masked gating control.
- Controlled conditions: `rom_alu` and `rom_mem_vdp` use an explicit
  `--instructions-per-line 1000000` to keep their short measured windows
  interrupt-free. `rom_masked` **omits** the flag, runs at the CLI default quantum of
  64, contains no SR-writing opcode (so both sides stay at mask 7), executes 155
  instructions and crosses HBlank boundaries at ticks 64 and 128 with no handler
  entry. This masked pass is negative evidence for gating, not an interrupt-delivery
  comparison. Every ROM initialises the compared data/address registers it uses and
  the reset vector sets the same stack pointer, because the oracle's reset D/A
  values are undefined; VDP control/data-port writes are executed but VDP data-port
  reads are excluded because oracle prefetch differs from this VDP model.
- Current result: **PASS 3/3** (3/3 trace, 3/3 final state, 3/3 negative controls,
  3/3 scheduler controls, exit 0). `rom_alu`: 2375 oracle total / 99 projected,
  99/99 ChronoFork, projection SHA-256 `2b6f191c…` on both sides, negative control
  index 49. `rom_mem_vdp`: 2363 / 51, 51/51, `ba4f3948…`, negative control index 25.
  `rom_masked`: 2388 / 155, 155/155, `02891e15…`, negative control index 77; quantum
  64 (CLI default, flag omitted), SR mask 7 on both sides, two HBlank boundaries
  crossed and zero handler entries.
- History: the pre-repair run passed `rom_alu` but `rom_mem_vdp` diverged at
  projected index 28 (`0x00027A`, opcode `0x2239`, `MOVE.L $00FF0020,D1`) because
  the core decoded the absolute-word (`0x..38`) form for absolute-long loads. The
  real encoding is `0x..39`; E2-13 repaired the decoder and the post-repair run is
  current evidence. The pre-repair artifacts are preserved under ignored
  `.research/evidence/before-repair/`. The earlier canonical two-ROM Phase-10 run at
  the explicit 1,000,000-instruction quantum is preserved unchanged under ignored
  `.research/evidence/before-e2-15/` as controlled-condition history.

**This is not cycle, interrupt-timing or interrupt-delivery equivalence.** The
masked-ROM pass shows that no handler executes while the SR mask is 7 and that the
plain instruction stream and selected final state agree; it does not compare
delivery order, acknowledge semantics, latency, vector fetch or handler execution
against the oracle. It also does not compare timing, prefetch, bus cycles, rendering,
audio, Z80 or DMA, and it does not establish general 68000 or game compatibility.

### Known limitations of the integration

- **Privilege is not enforced.** `MOVE.W #imm,SR` and `RTE` execute in either mode;
  `STOP`, `ORI`/`ANDI`/`EORI` to `SR` and separate user/supervisor stack pointers
  are absent.
- **Only VBlank (level 6) and HBlank (level 4) sources are modelled.** The "level 7
  is always deliverable" branch is a tested predicate rule in the shared
  68000/scheduler API, not a wired level-7 interrupt source. Other exception groups
  (timers, Z80, address/bus errors as 68000 exceptions, `TRAP`, etc.) are absent;
  unsupported instructions and bus faults surface as typed Rust errors, not 68000
  exception vectors.
- **No interrupt-timing equivalence is claimed.** Delivery happens at instruction
  boundaries in the instruction-quantum model (one instruction equals one tick), at
  most one exception per boundary; the oracle differential evidence does not compare
  delivery order, latency, acknowledge semantics, vector fetch or handler execution.
- **VINT pending/acknowledge semantics are minimal.** Status bit 7 is driven only by
  the scheduler's pending VBlank request. VDP interrupt-enable registers, VDP-side
  acknowledge logic, controller-port interrupt acknowledge and other VDP interrupt
  sources are not modelled. A status-port read never acknowledges a request; only
  backend delivery does. Only VBlank (bit 3), derived VINT pending (bit 7) and
  FIFO-empty (bit 8) can change; all other status bits are documented constants.
- **Pending requests are one per level.** A masked request is retained with its
  original issue tick and repeated requests of the same level coalesce (diagnostic
  counters record coalescing); there is no interrupt queue.
- Reset CCR/X differs before ROM arithmetic: the oracle powers on with SR `0x7FFF`
  (CCR `0x1F`, `X=1`), ChronoFork resets to `0x2700` (CCR `0`), and `MOVE` preserves
  `X`. A MOVE-only differential window can therefore differ in `X` while
  `N`/`Z`/`V`/`C` match; the harness ROMs clear `X` with arithmetic, so their
  recorded final effective-SR equality remains valid.

### Tests

`chronofork-megadrive` contributes unit and integration tests covering:

- reset vectors and register initialization;
- every implemented 68000 instruction form, flags, `DBRA` termination and
  interrupt-entry success/error paths;
- the 68000 mask rule (`interrupt_level_deliverable` / `accepts_interrupt_level`):
  level 7 always admissible and every other level strictly above the IPM; invalid
  levels rejected before state changes;
- interrupt entry loading the accepted level into the handler IPM, including
  level 7, while preserving the CCR; `MOVE.W #imm,SR` writing the raw `SR`/IPM; and
  `RTE` restoring raw `SR`, mode and CCR, PC and A7 while rejecting underflow,
  unmapped frames and odd PCs without touching state;
- ROM/RAM big-endian access, alignment, unmapped and atomic-write errors;
- VDP port mapping through the bus, register/VRAM round-trips, byte-lane behavior,
  side-effect-free debugger reads and explicit PSG/Z80/controller errors;
- VDP snapshot round-trip and malformed-input rejection; VDP VINT-pending driven by
  the owning machine, status reads never clearing it, and the derived latch being
  excluded from the VDP snapshot;
- deterministic tick-driven HBlank/VBlank generation, coalescing, priority,
  scanline timing and scheduler snapshot rejection; `next_deliverable` reflecting
  the 68000 mask;
- VDP register/VRAM programming by the synthetic 68000 program through the bus;
- masked HBlank retention until the program lowers the mask, delivery of only the
  delivered level, VBlank-before-HBlank delivery, and the masked request surviving
  with its issue tick;
- VBlank handlers at IPM 6 blocking pending-HBlank re-entry until `RTE` restores
  the program mask, followed by exactly one later HBlank delivery;
- masked VBlank keeping VINT pending across many steps without acknowledging,
  status-port reads not acknowledging masked requests, and VBlank acknowledge
  clearing only the VBlank request;
- snapshot/restore including VDP and pending-interrupt state, derived VINT
  recomputation, old/malformed payload rejection and configuration mismatch;
- session fork/restore with VDP and pending-interrupt state, including forking a
  masked request and delivering it after the mask is lowered, plus source isolation;
- device-level determinism across two independent machines;
- controller data-register reads: the exact `0xA10003`/`0x00A10005` byte map, active-low
  TH-high bit values, two independent ports, neutral `0x7F`, A/Start/high-bit inertness,
  typed writes and debugger rejection, step-scoped clearing on success and failure, no
  controller field in the snapshot payload/state/config hashes, checkpoint replay with the
  next supplied frame, and `controller_rom()` observed work-RAM values (6 integration tests
  in `tests/controller.rs`, plus in-crate unit/property tests in `snapshot_properties.rs`);
- bounded trace semantics: zero-capacity rejection, eviction/dropped/index
  reporting, clear/take, illegal-opcode recording, `MOVE.W #imm,SR`/`RTE`
  recording, and byte-identical snapshots, state hashes and payload versions with
  tracing on and off.

CLI integration tests cover deterministic trace and state files across two runs,
`--stop-pc`, stdout trace mode, a zero instruction budget, option validation, help
text, the default scheduler quantum staying 64 and never being hidden, explicit
quantum recording, and deterministic `megadrive demo` output including masked
HBlank/VBlank retention, VINT status, mask lowering, handler IPM, `RTE` restoration
and no nested HBlank handler. The `tas_hardware.rs` CLI process target (13 tests) covers
hardware record/play/verify/edit/studio with `--rom`, two-controller v2 input, the
synthetic-default regression and the typed ROM/backend errors. `cargo test --workspace`
is the source of truth for the current count.

## Implemented: controller-port input for TAS

[ADR-0009](../adr/0009-real-hardware-input-tas.md) is accepted, implemented and
independently verified for real-hardware input TAS (t5 PASS, 2026-11-16; M3-A45–A53), on
the deterministic minimal-slice scope and with the canonical Mega Drive state unchanged.

- Byte reads of `0xA10003` (controller 1) and `0xA10005` (controller 2) are driven by
  the current step's `InputFrame` ports 0 and 1. Only the TH-high phase of a 3-button
  pad is represented: bits 0–5 are Up/Down/Left/Right/B/C active-low, bit 6 is the
  TH-high level (1) and bit 7 is 0; a neutral port reads `0x7F`. A and Start require
  the TH-low phase and are **not** readable; the A/Start bits in the frame are inert.
- The input is transient and per-step: it is not serialized into the snapshot, not
  part of the state body, state hash or configuration hash, and cleared after every
  step. A restored machine uses the next step's input. No canonical Mega Drive byte
  changes; the differential payload/envelope/meta probe, the golden snapshot/state
  assertions, the demos, the oracle/state probes and all existing M2/M3 movie fixtures are
  byte-identical to the pre-change baseline.
- Every other controller-window access, including all writes and the version,
  control, port-3 and TX/RX addresses, stays a typed `UnmappedRead`/`UnmappedWrite`;
  the side-effect-free debugger path does not map step-scoped input.
- The CLI exposes `--rom` on `tas record`/`play`/`verify`/`studio`/`edit` and keeps
  `compare` backend-agnostic. The synthetic default path and its bytes are unchanged.
- Acceptance evidence is the [M3 hardware acceptance
  plan](../quality/m3-hardware-acceptance.md) rows M3-A45–A53, `executed - passed`
  (t5, 2026-11-16), and the ignored `.research/m3-hardware/verification/` artifacts. No
  controller, TH/6-button/Start, timing, commercial-ROM or accuracy claim is made.

## Not implemented

- VDP rendering, planes, sprites, scrolling, CRAM/VSRAM, DMA and raster timing.
- YM2612, PSG or audio.
- Z80, bus arbitration, timer/other interrupt sources, controller-port interrupt
  acknowledge, other 68000 exception groups and privilege enforcement.
- Controller input beyond the TH-high 3-button subset: A and Start, the TH latch/direction
  register and TH-low phase, 6-button pad (X/Y/Z/Mode), controller timing/interrupts, port 3,
  and analog/keyboard hardware. The implemented data-register reads are described above and
  in [ADR-0009](../adr/0009-real-hardware-input-tas.md).
- Cartridge mapper/SSF2, SRAM, region/TMSS behavior.
- 68000 prefetch, bus-cycle timing, separate user/supervisor stacks, most
  addressing modes and most instructions.
- Cycle accuracy. One executed instruction equals one ChronoFork tick in this
  slice; the scheduler maps instruction quanta to scanlines with configurable
  constants. This is deterministic but not hardware timing.
- ROM header validation, checksum, region detection or ROM database.
- ROM header validation, checksum, region detection or a ROM database. The
  `megadrive trace` and TAS `--rom` commands accept an explicitly supplied image; the demo
  uses an independently authored synthetic ROM.
- General structured/timestamped tracing or bus/device traces, historical trace
  queries, and a reusable in-product trace-comparison API. Only the narrow debug TSV
  and the external oracle harness exist.
- Differential evidence beyond three synthetic ROMs, positive differential
  comparison of interrupt delivery/timing, and any differential claim about
  interrupts as hardware behavior, VDP data reads, VRAM contents, audio, Z80, DMA,
  timing or game compatibility. The masked-ROM run is negative gating evidence
  only.

Unsupported opcodes and port accesses return errors; they are never silently
skipped.

## CLI demo

```sh
cargo run -p chronofork-cli -- megadrive demo
```

The demo executes `1 + 2`, programs VDP register `0x0F` (auto-increment 2), writes
and reads back VRAM word `0x1234`, and captures the VDP status. With the default
quantum 64 and the reset mask 7 it shows an HBlank request pending at tick 64 with
no handler at tick 65, then demonstrates snapshot/fork/restore with the pending
request preserved. At a shorter demonstration quantum it shows a masked VBlank at
tick 896 with VDP VBlank and VINT-pending status bits set and restored from a
checkpoint. Using the independently authored interrupt ROM it shows
`MOVE.W #$2300,SR` lowering the mask to 3 at tick 8, the retained HBlank delivered
at tick 9 with the handler at IPM 4, and `RTE` at tick 11 returning to PC
`0x000214` with raw `SR` `0x2300` (mask and CCR restored). VBlank is then entered at
IPM 6 before the pending HBlank, so no HBlank handler nests before `RTE` restores the
program mask; the checkpoint preserves the pending VBlank/VINT state. It prints
`chronofork megadrive demo: PASS`.

## CLI instruction trace

```sh
cargo run -p chronofork-cli -- megadrive trace \
  --rom <rom-path> --state-out <state-path> \
  --instructions 10000 --stop-pc 0x352 \
  --instructions-per-line 1000000 --ram-bytes 64 \
  --trace-out <trace-path>
```

`megadrive trace` is the evidence-producing CLI surface. It runs a decimal
instruction budget (or stops before `--stop-pc`), records a bounded ring of
executed `index`/`pc`/`opcode` records and writes the TSV trace plus a deterministic
key=value final state. `--trace-capacity` bounds the ring (default: the instruction
budget, capped); `--video` selects NTSC/PAL scheduler constants; `--trace-out`
defaults to stdout; `--state-out` is required. The trace is debug data: it does not
change snapshots, state hashes or the payload version.

Interrupt delivery is gated by the 68000 SR interrupt-priority mask, so a
differential window that must be interrupt-free can either use a program that never
writes `SR` (the reset mask 7 suppresses VDP levels) or pass a large explicit
`--instructions-per-line` (for example `1000000`). The default remains 64 and is
never changed implicitly; the CLI records the effective quantum in the state file
and prints a note on stderr when the requested instruction window exceeds the
configured quantum, describing the instruction-quantum model and SR-mask gating.

## Next increments

1. ROM header/region/config identity and a cartridge loader (EPIC-0002 E2-11).
2. More 68000 addressing modes, instructions and exception groups (EPIC-0002
   E2-13 continued), driven by differential tests.
3. Broaden the differential corpus beyond the three synthetic ROMs while keeping
   address/state comparison distinct from cycle/timing equivalence. A positive
   comparison of interrupt delivery or timing would need an oracle model of those
   semantics; the current masked-ROM pass is negative gating evidence only.
4. Privilege enforcement and separate user/supervisor stacks; additional interrupt
   sources (a wired level-7 source, timers, VDP interrupt-enable registers) if the
   slice grows to model them.
5. Deeper controller protocol (TH latch/TH-low phase, A/Start, 6-button pad, port 3) only
   if the slice grows to model it; the TH-high 3-button reads and `--rom` hardware TAS are
   implemented and verified under [ADR-0009](../adr/0009-real-hardware-input-tas.md).
6. Rendering/DMA only after the non-rendering state model is stable.
