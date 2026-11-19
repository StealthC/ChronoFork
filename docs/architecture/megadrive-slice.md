# Mega Drive 68000 Slice

- **Status:** Partial real-hardware slice implemented (68000 + ROM/RAM + VDP port
  window + VBlank/HBlank scheduler with SR-mask gating, `RTE` and VDP
  VINT-pending/acknowledge + bounded debug instruction trace, on the master-cycle
  timeline, with eleven-ROM oracle differential evidence: address projections,
  per-instruction cycle costs and selected final states all match 11/11); not a
  complete emulator. Controller-port data-register reads for real-hardware
  input TAS are implemented and independently verified for the TH-high 3-button subset on
  ports 1–2 ([ADR-0009](../adr/0009-real-hardware-input-tas.md); t5 PASS, 2026-11-16), with
  the canonical state, demos and M1 evidence unchanged. A/Start, TH/6-button/Start
  multiplexing, controller timing, analog/keyboard hardware and port 3 remain absent.
  M4 debugging depth (bounded non-canonical observation, views/disassembly and
  CPU/VDP/interrupt/bus inspectors) is implemented and independently verified for this
  slice (t5 PASS, 2026-11-16). M8 phase 8.1 adds a modelled cartridge — header/region/
  checksum, Sega mapper, 64 KiB battery SRAM, TMSS lockout and version register — with
  payload/state/config v3 (`CFMD68K3`/`CFMDSTATE3`/`CFMDCONF3`; implemented, reviewed and
  verified 2026-11-17). Phase 8.2 moves the identity to v4
  (`CFMD68K4`/`CFMDSTATE4`/`CFMDCONF4`) with the master-cycle timeline and extended CPU
  snapshot fields; booting a commercial title is phase 8.6 evidence, not claimed here.
- **Current milestone:** [M8 — Functional Mega Drive](../roadmap/milestones.md#m8--functional-mega-drive)
  is **in progress**. Phase 8.2 (the documented MC68000 set and master-cycle timeline,
  with 11/11 address, per-instruction cycle and final-state differential evidence) is
  implemented; verification passed (round 12), with review and merge pending. M8 then completes the
  remaining slice (Z80, VDP rendering/DMA/timing, YM2612/PSG audio, cartridge,
  controllers and capture). This page remains the canonical capability/limitation home;
  it is updated as each phase lands, never in advance.
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
| Cartridge ROM | `0x000000..0x3FFFFF` | Immutable, big-endian; bank-mapped through the Sega mapper; only image bytes are valid |
| Mapper registers | `0x00A130F0..0x00A130FF` | Write-only: `0xA130F1` is the SRAM control byte, odd registers `0xA130F3..0xA130FF` select the seven bank windows |
| Battery SRAM | `0x00200000..0x0020FFFF` | 64 KiB, mapped over the ROM window while `0xA130F1` bit 0 is set; bit 1 write-protects it |
| TMSS register | `0x00A14000..0x00A14003` | A long write whose bit 9 is set releases the enabled VDP-write lockout |
| Version register | `0x00A10001` | Cartridge-reported export/PAL/no-disk/hardware byte |
| Work RAM | `0xFF0000..0xFFFFFF` | 64 KiB, big-endian, mirrored by address bits 15–0 |
| VDP ports | `0xC00000..0xC0000F` | Data port `0x00..0x03`, control/status port `0x04..0x07`, HV counter `0x08..0x0F` |
| Controller data registers | `0x00A10003`, `0x00A10005` | Byte reads only, driven by the current step's `InputFrame` ports 0/1; see below |
| Everything else | — | Explicit `UnmappedRead` / `UnmappedWrite` error |

- Word and long accesses require even addresses.
- ROM writes are rejected; the mapper register file is write-only, so a read there is a
  typed `UnmappedRead`.
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

### 68000 instruction set

The Phase 8.2 implementation covers the documented MC68000 instruction groups:

| Group | Instructions |
|---|---|
| Data movement | `MOVE`/`MOVEA` (all documented source/destination modes), `MOVEQ`, `MOVEM`, `MOVEP`, `LEA`, `PEA`, `CLR`, `TST`, `EXT`, `SWAP`, `EXG` |
| Arithmetic | `ADD`/`ADDA`/`ADDQ`/`ADDI`/`ADDX`, `SUB`/`SUBA`/`SUBQ`/`SUBI`/`SUBX`, `CMP`/`CMPA`/`CMPI`/`CMPM`, `NEG`, `NEGX`, `MULU`, `MULS`, `DIVU`, `DIVS` |
| Logic | `AND`/`ANDI`, `OR`/`ORI`, `EOR`/`EORI`, `NOT` |
| Shifts/rotates | `ASL`, `ASR`, `LSL`, `LSR`, `ROL`, `ROR`, `ROXL`, `ROXR` in register and memory forms |
| BCD | `ABCD`, `SBCD`, `NBCD` |
| Bit operations | `BTST`, `BCHG`, `BCLR`, `BSET` (static and dynamic bit number), `TAS` (register and memory) |
| Control flow | `BRA`, `BSR` and every `Bcc` condition in 8- and 16-bit displacement forms, `DBcc`, `Scc`, `JMP`, `JSR`, `RTS`, `NOP`, `RTR` |
| System/privileged | `RTE`, `TRAP`, `TRAPV`, `CHK`, `ILLEGAL`, `LINK`, `UNLK`, `MOVE` to `SR`/`CCR` and from `SR` (MC68000), `MOVE` from `CCR` (68010 extension; see below), `MOVE` to/from `USP`, `ORI`/`ANDI`/`EORI` to `CCR`/`SR`, `STOP`, `RESET`, interrupt entry |

Every documented MC68000 instruction reports the instruction-boundary cycle cost
used by the oracle-backed timing table (see the timing and differential sections
below); effective-address cost is added exactly once per operand. One oracle-model
difference is recorded rather than hidden: the pinned oracle charges a flat 6
cycles for `MOVE from SR` to memory instead of a write-EA table, and ChronoFork
matches the oracle for the differential. `MOVE to CCR` (`0x44C0`) is a documented
MC68000 instruction and is exercised by the differential ROM; only `MOVE from CCR`
(`0x42C0`) is the 68010 addition. `MOVE from CCR` is implemented for compatibility
but is outside the MC68000 timing table and the differential corpus. Unrecognised
and reserved encodings raise the guest illegal-instruction exception (vector 4)
with the documented six-byte frame rather than a host error.

Reset loads the supervisor stack pointer from address `0x000000` and the program
counter from `0x000004`; `SR = 0x2700`, all general registers clear.

Flags implemented: `N`, `Z`, `V`, `C`, `X` for the documented arithmetic, logic,
shift, BCD and bit operations. `SR` upper bits are preserved.

The interrupt entry API pushes a word raw `SR` and a long return `PC` (six bytes),
sets the supervisor bit, preserves the CCR, loads the accepted interrupt level into
the interrupt-priority mask, reads the target long from the supplied vector-table
address and jumps to it. It wakes a halted processor. Invalid levels are rejected
before any state changes; stack or vector faults return typed errors (a failed first
push leaves `SR`/`PC` untouched, while the six-byte frame can already be pushed
before a later vector or target fault). `RTE` validates the complete frame before
changing canonical state, restores the exact raw `SR` (IPM, `S`, CCR), restores the
PC and advances A7 by six. `MOVE.W #imm,SR` (`0x46FC`) replaces the raw `SR` so a
program can change its mask. Privileged operations (`MOVE` to `SR`, `STOP`, `RESET`,
`RTE`, `MOVE` to/from `USP`) raise the privilege-violation exception (vector 8) in
user mode. Guest exception vectors actually delivered are address error (3), illegal
instruction (4), zero divide (5), `CHK` (6), `TRAPV` (7), privilege violation (8),
autovector levels 4 and 6 (28/30) and `TRAP #n` (32+n); vector 1 is the reset
vector, and bus-error (2) and trace (9) exception delivery are not implemented.

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

- Master-cycle model: canonical time is the master-clock cycle. Each executed
  instruction advances the backend by its 68000 cycle cost times seven, and one
  scanline is exactly 3420 master cycles (`MASTER_CYCLES_PER_LINE`). This is
  explicitly **not** fully cycle-accurate hardware timing: functional prefetch,
  bus contention, DMA stalls and interrupt latency are not modelled. The former M3
  instruction-quantum model is superseded.
- NTSC constants (262 lines/frame, 224 active) and PAL constants (313/240), with
  the fixed 3420-master-cycle scanline; `SchedulerConfig::new(video)/ntsc()/pal()`
  carries only the video standard.
- HBlank is requested when each active scanline completes at a master-cycle
  boundary; VBlank when vertical blanking begins. Blanking lines are quiet.
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
  ROM, video standard and scheduler configuration into `config_hash` (`CFMDCONF4`).
- `MegadriveBackend` implements the existing `Backend` contract: execution,
  snapshots, restore, inspection, memory reads/writes.
- Snapshot payload magic is `CFMD68K4`, payload version **4**, followed by the
  68000 register file (including `USP`, `SSP`, the 68000 cycle counter and the
  `STOP` latch), `PC`, `SR`, halted flag, tick, 64 KiB RAM, the complete VDP snapshot
  and the complete master-cycle scheduler snapshot. Versions 1-3 are rejected rather
  than reinterpreted.
- The state hash (`CFMDSTATE4`) covers the extended 68000 state, tick, RAM, VDP
  snapshot and scheduler snapshot, so pending interrupts, stack-pointer mirrors and
  VRAM changes are part of checkpoint identity.
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
- Capabilities truthfully report `breakpoints = false` and `tracing = false`: host
  breakpoints are a debugger feature and the bounded trace is a concrete-type debug
  facility, not the generic `Backend` `tracing` capability. Since M4 the slice reports
  `watchpoints = true` and `debug_events = true` for its optional observation layer, and
  `rendering = false` / `audio = false`.

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
  writes TSV `index<TAB>pc_hex<TAB>opcode_hex` (and, with `--cycles`, a fourth
  decimal 68000-cycle field) and a deterministic key=value final state (executed,
  tick/master_cycles, cpu_cycles, USP/SSP, video, halted, D0–D7, A0–A7, PC, raw
  SR, RAM base/length/hex), and honours `--video`, `--trace-capacity`, `--out`/
  `--trace-out` (default stdout) and required `--state-out`. The instruction-quantum
  flag is removed.

### Oracle differential method and limits

The reproducible harness `scripts/oracle-diff.py` compares the minimal core against
a pinned Exodus headless oracle for eleven independently authored synthetic ROMs; the
full numbers, commands and limitations are in the
[68000 cycle differential evidence](../research/cpu-cycle-differential-evidence.md).

- Both sides produce an instruction trace; the harness projects each to the ordered
  sequence of executed instruction **addresses** before a marker self-loop and
  compares the per-instruction 68000 cycle cost (`cycle` column delta versus
  ChronoFork's fourth trace field). Because the oracle renders
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
- Phase 8.2 compares the master-cycle timeline: the oracle's accumulated `cycle`
  column difference per instruction is checked against ChronoFork's fourth trace
  field (`InstructionOutcome.cycles`, emitted by `--cycles`). Absolute cycle bases
  and the oracle `time` column are not compared. M3's per-ROM instruction quantum
  and masked-boundary control are superseded.
- The eleven synthetic ROMs are the three historical ones plus EA, shift, BCD,
  MOVEM/MOVEP, mul/div, EXG/TAS/ADDX/SUBX, system/status-register and bit/DBcc
  fixtures under `scripts/fixtures/`. The regenerated repair run passes: address projections
  11/11, per-instruction cycle comparisons 11/11, selected final states 11/11, and
  address/cycle/final-state negative controls 11/11; the harness exits 0. No
  divergence remains in the compared observables and no expectation was edited to
  obtain the pass. Full numbers, commands and the pinned
  oracle revision `ec005615…` are in the
  [cycle differential evidence](../research/cpu-cycle-differential-evidence.md).
- History: the pre-repair address divergence at `0x00027A` (absolute-long decode)
  remains recorded in the historical
  [oracle differential evidence](../research/oracle-differential-evidence.md); the
  M3 quantum/masked-boundary results there are marked superseded for timing.

**This is not cycle, interrupt-timing or interrupt-delivery equivalence.** The
masked-ROM pass shows that no handler executes while the SR mask is 7 and that the
plain instruction stream and selected final state agree; it does not compare
delivery order, acknowledge semantics, latency, vector fetch or handler execution
against the oracle. It also does not compare timing, prefetch, bus cycles, rendering,
audio, Z80 or DMA, and it does not establish general 68000 or game compatibility.

### Known limitations of the integration

- **Privilege is enforced for the implemented system instructions.** `MOVE` to
  `SR`, `STOP`, `RESET`, `RTE` and `MOVE` to/from `USP` raise vector 8 in user mode;
  the exception framework provides vectors 3-8/24-31/32+n, short frames and the
  documented 14-byte address-error frame. The documented MC68000 instruction set
  is implemented; guest bus-error (vector 2) and trace (vector 9) delivery,
  automatic trace mode and the remaining exception-group edge cases are absent.
- **Only VBlank (level 6) and HBlank (level 4) sources are modelled.** The "level 7
  is always deliverable" branch is a tested predicate rule in the shared
  68000/scheduler API, not a wired level-7 interrupt source. Other exception groups
  (timers, Z80, bus-error recovery and trace) are absent. Unmapped and
  write-protected guest data accesses take the guest address-error exception
  (vector 3) with the documented 14-byte frame and function code; odd/misaligned EA
  accesses, unrecognised/reserved encodings and the implemented
  `TRAP`/`TRAPV`/`CHK`/`ILLEGAL` forms also use 68000 vectors. Host errors are
  reserved for host-level failures (a frame or vector fetch that cannot complete,
  stack/address overflow, trace capacity), not for guest data accesses. Instruction
  and extension-word fetches are not converted: an unmapped or protected fetch
  remains a typed host bus error, which is the declared guest bus-error (vector 2)
  residual, distinct from the data accesses that take vector 3.
- **No interrupt-timing equivalence is claimed.** Delivery happens at instruction
  boundaries on the master-cycle timeline (cycle cost added at the boundary), at
  most one exception per boundary; the oracle differential does not compare delivery
  order, latency, acknowledge semantics, vector fetch or handler execution.
- **VINT pending/acknowledge semantics are minimal.** Status bit 7 is driven only by
  the scheduler's pending VBlank request. VDP interrupt-enable registers, VDP-side
  acknowledge logic, controller-port interrupt acknowledge and other VDP interrupt
  sources are not modelled. A status-port read never acknowledges a request; only
  backend delivery does. Only VBlank (bit 3), derived VINT pending (bit 7) and
  FIFO-empty (bit 8) can change; all other status bits are documented constants.
- **Pending requests are one per level.** A masked request is retained with its
  original issue tick and repeated requests of the same level coalesce (diagnostic
  counters record coalescing); there is no interrupt queue.
- Reset CCR/X is architecturally undefined: the oracle powers on with `X=1` and
  unused raw-SR bits set (raw `SR 0x7FE0`), while ChronoFork resets to raw `SR
  0x2700` (CCR `0`). Effective fields are compared and the harness ROMs clear `X`
  explicitly with `ADD.B D0,D0` before the measured section, so an unwritten reset
  flag cannot masquerade as a state divergence. Raw `SR` words are still not
  compared.

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
  side-effect-free debugger reads, mapped byte reads of the two controller data
  registers and explicit PSG/Z80 and other-controller-access errors;
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
text, the master-cycle timeline and removed instruction-quantum flag, and
deterministic `megadrive demo` output including masked HBlank/VBlank retention,
VINT status, mask lowering, handler IPM, `RTE` restoration and no nested HBlank
handler. The `tas_hardware.rs` CLI process target (13 tests) covers
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

## Implemented: cartridge, region and TMSS (M8 phase 8.1)

Phase 8.1 turns the immutable ROM blob into a modelled cartridge
([design](../superpowers/specs/2026-11-17-8.1-cartridge-region-tmss-design.md),
[plan](../superpowers/plans/2026-11-17-8.1-cartridge-region-tmss.md)). The
`chronofork-megadrive` cartridge device owns the image, the parsed header, the Sega mapper
register file, 64 KiB battery SRAM and the TMSS latch; the bus delegates every cartridge
window to it.

- **Header.** System signature at `0x100` (`SEGA MEGA DRIVE` / `SEGA GENESIS`; other
  signatures rejected), bounded ASCII domestic/overseas names and serial, stored checksum
  at `0x18E` with the computed Mega Drive **word sum over `0x200..`**, ROM start/end at
  `0x1A0`/`0x1A4`, the SRAM specification (`RA` marker at `0x1B0`, start `0x1B4`, end
  `0x1B8`) and the region bitfield at `0x1F0`. A missing or unknown region, a short image
  and a declared end beyond the image are typed errors; a checksum mismatch is diagnostic
  only.
- **Sega mapper.** Always present and identity by default, so images up to 4 MiB read
  linearly. Window 0 is fixed to bank 0; windows 1–6 at `0x080000 + i × 0x80000` use the
  nine-bit registers `0xA130F3..0xA130FF`. A selected page beyond the image is a typed
  `UnmappedRom`; odd word/long accesses and ROM-window writes are typed errors.
- **Battery SRAM.** 64 KiB canonical state, zeroed at construction, mapped only while
  `0xA130F1` bit 0 is set and write-protected by bit 1 (a rejected write mutates no byte).
  `import_sram` is the explicit construction-time host save-file seam and requires exactly
  64 KiB; the CLI imports and exports raw 64 KiB images.
- **TMSS.** Modelled and **disabled by default**. `Enabled` blocks VDP writes until a long
  write whose bit 9 is set reaches `0xA14000`; while locked a VDP write is acknowledged and
  discarded, leaving the VDP registers and its address latch untouched. `0xA10001`
  reports `(export << 7) | (pal << 6) | no-disk | tmss-capable`.
- **Video standard.** Derived from the header (PAL only for an `E`-only region) with an
  explicit `MegadriveConfig { video, tmss }` override for tests and region-mismatched
  dumps. Both are configuration, never mutable state.
- **State (current, after phase 8.2).** Payload v4 (`CFMD68K4`), state hash v4
  (`CFMDSTATE4`) and configuration hash v4 (`CFMDCONF4`). Phase 8.1 used v3
  (`CFMD68K3`/`CFMDSTATE3`/`CFMDCONF3`); phase 8.2 moved the identity to v4 for the
  master-cycle timeline and extended 68000/scheduler state. The snapshot adds the seven
  mapper banks, the SRAM control byte, 64 KiB SRAM, the TMSS model/lock latch and the
  extended 68000/scheduler state; v1-v3 payloads are rejected, a payload carrying another
  TMSS model is rejected even when its hash was recomputed, and a rejected restore leaves
  the machine untouched. The configuration hash
  covers the ROM SHA-256, video standard, TMSS model and a mapper/SRAM feature tag; SRAM
  bytes are state and never configuration.
- **Inspection.** `CartridgeView` plus bounded `cartridge.*`, `mapper.bank0..6`, `sram.*`
  and `tmss.locked` pseudo-registers.
- **CLI.** `chronofork megadrive cartridge --rom <FILE> [--sram-in <FILE>]
  [--sram-out <FILE>]` prints the parsed facts, and `chronofork megadrive boot --rom <FILE>
  [--frames <N>] [--tmss enabled|disabled]` steps the synthetic boot cartridge to its
  marker with a bounded frame budget (non-zero exit when the marker is not reached).
- **Evidence.** The independently authored 2 MiB `cartridge_boot_rom()` unlocks TMSS,
  enables SRAM, selects bank 2, writes SRAM and reaches `BOOT_MARKER`; the two curated
  fixtures (Kid Chameleon, Chakan) are parsed and their system/region/serial/checksum
  pinned when the ignored `.research/roms/` files are present. Booting a commercial title
  to its main loop is **not** claimed here — that is phase 8.6.

## Not implemented

- VDP rendering, planes, sprites, scrolling, CRAM/VSRAM, DMA and raster timing.
- YM2612, PSG or audio.
- Z80, bus arbitration, timer/other interrupt sources and controller-port interrupt
  acknowledge. Guest bus-error (vector 2) and trace (vector 9) exception delivery are
  not implemented; privileges are enforced for the implemented system instructions.
- Controller input beyond the TH-high 3-button subset: A and Start, the TH latch/direction
  register and TH-low phase, 6-button pad (X/Y/Z/Mode), controller timing/interrupts, port 3,
  and analog/keyboard hardware. The implemented data-register reads are described above and
  in [ADR-0009](../adr/0009-real-hardware-input-tas.md).
- Cartridge mapper variants beyond the Sega banking above (interleaved dumps, 32X/Pico
  hardware, images above the 64 MiB mapper ceiling) and SRAM banking schemes other than the
  64 KiB window.
- 68000 prefetch, bus-cycle timing, bus contention, DMA stalls and interrupt
  latency. The instruction set and documented addressing modes are implemented, and
  the register file carries separate user/supervisor stacks.
- The Mega Drive TAS read-modify-write write-back quirk (some consoles suppress the
  write-back phase) is a bus/hardware behaviour, not a 68000-core behaviour; the
  core performs the documented write. The oracle fixture therefore exercises the
  TAS register forms and the memory write is covered by unit tests.
- Hardware cycle accuracy. Each instruction advances the master-cycle timeline by
  its documented 68000 cycle cost times seven; cost is added at the instruction
  boundary rather than distributed across internal microcycles. This is
  deterministic instruction-level timing, not a bus-cycle model.
- A ROM database, region-mismatch handling beyond the derived standard and its explicit
  override, or any commercial-ROM boot. Header/checksum/region parsing and the Sega mapper
  are implemented; the `megadrive trace` and TAS `--rom` commands accept an explicitly
  supplied image, and the demo uses an independently authored synthetic ROM.
- General structured/timestamped tracing or bus/device traces, historical trace
  queries, and a reusable in-product trace-comparison API. Only the narrow debug TSV
  and the external oracle harness exist.
- Differential evidence beyond the eleven synthetic ROMs, positive differential
  comparison of interrupt delivery/timing, and any differential claim about
  interrupts as hardware behavior, VDP data reads, VRAM contents, audio, Z80, DMA,
  full-hardware timing or game compatibility. The 11-ROM run matches instruction
  addresses, per-instruction cycle costs and selected final state; the masked-ROM
  run is negative gating evidence only.

Unrecognised and reserved opcodes raise the guest illegal-instruction exception;
unmapped port accesses return typed errors. Neither is silently skipped.

## CLI demo

```sh
cargo run -p chronofork-cli -- megadrive demo
```

The demo executes `1 + 2`, programs VDP register `0x0F` (auto-increment 2), writes
and reads back VRAM word `0x1234`, and captures the VDP status. On the master-cycle
timeline with the reset mask 7 it shows the first HBlank request pending at tick
3472 with no handler, then demonstrates snapshot/fork/restore with the pending
request preserved. A mask-7 interrupt ROM reaches masked VBlank with the VDP
VBlank/VINT-pending status bits set and restored from a checkpoint. The mask-3
interrupt ROM delays past two scanlines, lowers the mask at tick 7378, delivers the
retained HBlank at tick 7406 with the handler at IPM 4, and `RTE` at tick 7574
returns to PC `0x000210` with raw `SR` `0x2300` (mask and CCR restored). VBlank is
then entered at IPM 6 before the pending HBlank, so no HBlank handler nests before
`RTE` restores the program mask; the checkpoint preserves the pending VBlank/VINT
state. It prints `chronofork megadrive demo: PASS`.

Timing note: the round-4 system-timing correction moved the interrupt ROM's
retained-HBlank line from tick 3472 to tick 3458; the demo's first masked-HBlank
line remains 3472.

## CLI instruction trace

```sh
cargo run -p chronofork-cli -- megadrive trace \
  --rom <rom-path> --state-out <state-path> \
  --instructions 10000 --stop-pc 0x352 --cycles \
  --ram-bytes 64 --trace-out <trace-path>
```

`megadrive trace` is the evidence-producing CLI surface. It runs a decimal
instruction budget (or stops before `--stop-pc`), records a bounded ring of
executed `index`/`pc`/`opcode` records (plus a fourth 68000-cycle field with
`--cycles`) and writes the TSV trace plus a deterministic key=value final state.
`--trace-capacity` bounds the ring (default: the instruction budget, capped);
`--video` selects NTSC/PAL timeline constants; `--trace-out` defaults to stdout;
`--state-out` is required. The trace is debug data: it does not change snapshots,
state hashes or the payload version.

Interrupt delivery is gated by the 68000 SR interrupt-priority mask, so a
differential window that must be interrupt-free uses a program that never writes
`SR` (the reset mask 7 suppresses VDP levels). There is no instruction-quantum flag:
canonical time is the 3420-master-cycle scanline, and the state file records
`tick`/`master_cycles` and the accumulated 68000 `cpu_cycles`.

## Debugging depth (M4, implemented and verified)

[ADR-0010](../adr/0010-debugging-depth.md) and the [M4 acceptance
plan](../quality/m4-debugging-depth-acceptance.md) implement a debug-only observation
layer for this slice: an optional bounded `DebugAccess` event ring (watchpoint hits, RAM
writes, VDP port and controller reads, interrupt request/delivery/acknowledgement), pure
CPU/VDP/interrupt/bus view types, a bounded 68000-subset disassembler whose instruction
lengths are cross-checked against execution, and the `chronofork debug` CLI surface.
Debug state is absent from the canonical payload, state hash and configuration hash; the
independent verifier's 1,700-step off/on probe found zero canonical differences, demo and
movie bytes stayed identical, and every M4-A1–A17 row is `executed - passed` (t5 PASS
2026-11-16). Stops are at instruction boundaries; no mid-instruction, cycle-exact or
reverse execution is claimed. Bus events are byte-granular, VDP events carry the device
width, instruction fetches appear only through a matching read watchpoint, and the logpoint
ring is fixed at 256 entries with a 4,096 compile-time bound.

## Next increments

1. Boot a curated commercial title to its main loop with video and audio, and record the
   compatibility matrix (phase 8.6). The cartridge device, header identity, Sega mapper,
   SRAM and TMSS landed in phase 8.1.
2. Guest bus-error and trace exception delivery (vectors 2 and 9) and the
   remaining exception-edge behaviour, driven by differential tests.
3. Broaden the differential corpus beyond the eleven synthetic ROMs while keeping
   instruction-address/cycle/state comparison distinct from hardware timing
   equivalence. A positive comparison of interrupt delivery or latency would need
   an oracle model of those semantics; the current masked-ROM pass is negative
   gating evidence only.
4. Additional interrupt sources (a wired level-7 source, timers, VDP
   interrupt-enable registers) if the slice grows to model them.
5. Deeper controller protocol (TH latch/TH-low phase, A/Start, 6-button pad, port 3) only
   if the slice grows to model it; the TH-high 3-button reads and `--rom` hardware TAS are
   implemented and verified under [ADR-0009](../adr/0009-real-hardware-input-tas.md).
6. Rendering/DMA only after the non-rendering state model is stable.
