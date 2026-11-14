# Mega Drive 68000 Slice

- **Status:** Partial real-hardware slice implemented (68000 + ROM/RAM + VDP port
  window + interrupt scheduler + bounded debug instruction trace, with passing
  two-ROM oracle differential evidence); not a complete emulator.
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
a pinned Exodus headless oracle for two independently authored synthetic ROMs; the
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
- Controlled conditions: an explicit `--instructions-per-line 1000000` keeps the
  measured window interrupt-free because ChronoFork has no SR mask gating yet
  (E2-15); both ROMs initialise every compared data/address register they use and
  the reset vector sets the same stack pointer, because the oracle's reset D/A
  values are undefined; VDP control/data-port writes are executed but VDP
  data-port reads are excluded because oracle prefetch differs from this VDP model.
- Final result: both ROMs pass. `rom_alu` has 2375 oracle total / 99 projected
  records and 99/99 ChronoFork projected records with projection SHA-256
  `2b6f191c…` on both sides; `rom_mem_vdp` has 2363 / 51 and 51/51 with
  `ba4f3948…` on both sides. Final D0–D7, A0–A7, PC, effective SR and 64 work-RAM
  bytes match on both ROMs, and negative controls catch indices 49 and 25 with
  exit 1. Every projected ChronoFork opcode word also equals the ROM word at that
  address.
- Pre-repair history: `rom_alu` already passed, but `rom_mem_vdp` diverged at
  projected index 28 (`0x00027A`, opcode `0x2239`, `MOVE.L $00FF0020,D1`) because
  the core decoded the absolute-word (`0x..38`) form for absolute-long loads. The
  real encoding is `0x..39`; E2-13 repaired the decoder and the post-repair run is
  the final evidence. The pre-repair artifacts are preserved under ignored
  `.research/evidence/before-repair/`.

**This is not cycle equivalence.** It does not compare timing, prefetch, bus
cycles, interrupt delivery, rendering, audio, Z80 or DMA, and it does not establish
general 68000 or game compatibility.

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
- device-level determinism across two independent machines;
- bounded trace semantics: zero-capacity rejection, eviction/dropped/index
  reporting, clear/take, illegal-opcode recording, and byte-identical snapshots,
  state hashes and payload versions with tracing on and off.

CLI integration tests cover deterministic trace and state files across two runs,
`--stop-pc`, stdout trace mode, a zero instruction budget, option validation and
help text. `cargo test --workspace` is the source of truth for the current count.

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
- General structured/timestamped tracing or bus/device traces, historical trace
  queries, and a reusable in-product trace-comparison API. Only the narrow debug TSV
  and the external oracle harness exist.
- Differential evidence beyond two synthetic ROMs, and any differential claim about
  interrupts, VDP data reads, VRAM contents, audio, Z80, DMA, timing or game
  compatibility.

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

Because the core does not yet gate interrupts with the SR mask (E2-15), a
differential window that must be interrupt-free has to pass a large explicit
`--instructions-per-line` (for example `1000000`). The default remains 64 and is
never changed implicitly; the CLI prints a note on stderr when the requested
instruction window exceeds the configured quantum.

## Next increments

1. Wire the VDP VINT-pending status bit and controller-port interrupt acknowledge.
2. Add `RTE` and SR mask handling so handlers can return and masking can gate
   delivery.
3. ROM header/region/config identity and a cartridge loader.
4. More 68000 addressing modes and instructions, driven by differential tests.
5. Broaden the differential corpus and 68000 instruction/addressing coverage beyond
   the two current synthetic ROMs, keeping address-stream comparison distinct from
   cycle/timing equivalence.
6. Controller input schema (multi-port) before claiming input support.
7. Rendering/DMA only after the non-rendering state model is stable.
