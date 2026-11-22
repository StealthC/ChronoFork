# ADR-0009: Real-Hardware Input TAS — Controller-Port Reads and Mega Drive Backend Selection

- **Status:** **Accepted** (owner-approved 2026-11-15T23:38:06-03:00,
  America/Fortaleza; 2026-11-16T02:38:06Z); **implemented and independently verified** for
  the documented deterministic minimal-slice scope (t5 PASS, 2026-11-16). M3 is complete
  for that scope per the t6 promotion; the t7 promotion audit returned PASS on 2026-11-16
  (ignored `.research/m3-hardware/final-verification/SUMMARY.txt`).
- **Date:** 2026-11-15
- **Decision owner:** Project owner (approved 2026-11-15)
- **Approval evidence:** The project owner explicitly approved ADR-0009 as proposed with the
  statement "Aprovar ADR-0009 como proposto (Recomendado)" at
  2026-11-15T23:38:06-03:00 (America/Fortaleza), 2026-11-16T02:38:06Z. The approval covers
  the proposed contract as recorded in this ADR: stateless per-step controller reads at
  `$A10003`/`$A10005`; the 3-button active-low bit layout with bit 6 = 1, bit 7 = 0 and
  neutral `0x7F`; A/Start unreadable; fail-closed controller-window writes and
  debugger/`peek8` rejection; non-serialized input and no canonical snapshot/state-hash/
  demo/M1 change; transient bus wiring with no `Backend` trait or capability change;
  `--rom` on `record`/`play`/`verify`/`studio`/`edit` with `compare` backend-agnostic, a
  bounded ROM read, rejected `record --rom --root`, typed wrong-ROM/checkpoint errors and
  an unchanged synthetic default; the synthetic controller-ROM acceptance with
  snapshot/restore/fork/rewind/studio and two controllers; the deferred
  A/Start/TH/6-button/analog/keyboard/cycle-accuracy list; and no M3-completion claim
  before verification. The approval was a decision approval only; implementation and
  verification are recorded separately below and were not asserted by the approval.
- **Supersedes:** none. This ADR extends [ADR-0002](0002-backend-boundaries.md),
  [ADR-0004](0004-tas-movie-format.md),
  [ADR-0005](0005-partial-replay-and-branch-comparison.md),
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md),
  [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md) and
  [ADR-0008](0008-rerecord-and-branch-editing.md) without changing any canonical movie
  byte, format or schema version, limit, snapshot payload, state hash, configuration hash,
  scheduler behavior, emulation semantics, storage layout, existing CLI output, demo byte
  or previously verified M1/M2/M3 evidence.
- **Implementation status:** **implemented and independently verified** (t5 PASS,
  2026-11-16) for the deterministic minimal Mega Drive slice on Linux. The controller
  data-register reads, the step-scoped transient wiring, `controller_rom()`, the `--rom`
  selection on `record`/`play`/`verify`/`studio`/`edit`, and the controller and CLI test
  targets exist and passed. No controller capability bit, inspection
  pseudo-register, canonical payload/state-hash/config change or new dependency was added.
- **Verification status:** independent t5 verification PASS (2026-11-16) on HEAD
  `85af5238fabf204bbb885df3823c604212edd948` plus the increment working-tree paths. Every
  M3-A45–M3-A53 row is `executed - passed`; raw evidence is under the ignored
  `.research/m3-hardware/verification/` path (1057 artifacts, `MANIFEST.sha256` with
  `sha256sum -c` exit 0) and the dated PASS block is in
  [quality gates](../quality/quality-gates.md). No commit, push, tag or merge was made.
- **Related:** [Mega Drive 68000 slice](../architecture/megadrive-slice.md), [execution
  model](../architecture/execution-model.md), [backend model](../architecture/backend-model.md),
  [session model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [roadmap](../roadmap/roadmap.md), [M3 first-increment acceptance
  plan](../quality/m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](../quality/m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](../quality/m3-format-v2-acceptance.md), [M3 rewind acceptance
  plan](../quality/m3-rewind-acceptance.md), [M3 rerecord acceptance
  plan](../quality/m3-rerecord-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [oracle differential
  evidence](../research/oracle-differential-evidence.md), [anti-slop
  policy](../quality/anti-slop-policy.md), [quality gates](../quality/quality-gates.md).

> **Accepted, implemented and independently verified (t5 PASS, 2026-11-16).** The project
> owner approved this ADR on 2026-11-15T23:38:06-03:00 (America/Fortaleza; 02:38:06Z on
> 2026-11-16), and the increment was implemented and independently verified for the
> documented deterministic minimal-slice scope. The verified scope is stateless per-step
> Mega Drive controller data-register reads for ports 1–2 (TH-high 3-button subset), the
> transient non-serialized wiring, the synthetic `controller_rom()` acceptance fixture and
> `--rom` on `record`/`play`/`verify`/`studio`/`edit`, with `compare` backend-agnostic. The
> canonical Mega Drive snapshot payload (`CFMD68K2` v2), state body, state hash,
> configuration hash, demos, M1 oracle evidence and all existing M2/M3 movies are unchanged
> and byte-identical to the pre-change baseline. **M3 is complete for this documented
> minimal-slice scope**, recorded by the t6 promotion; the t7 promotion audit returned PASS
> on 2026-11-16 (ignored `.research/m3-hardware/final-verification/SUMMARY.txt`).
> Limits remain stated in §10: A and Start, TH/6-button/Start multiplexing, controller
> timing, analog/keyboard hardware, port 3, real commercial ROMs, cycle accuracy and
> import/export adapters are **not** implemented or claimed.

## Context

Read on 2026-11-15 in the `agent/m3-complete` checkout at HEAD
`85af5238fabf204bbb885df3823c604212edd948` (clean tree). A pre-change baseline for this
increment is recorded under the ignored `.research/m3-hardware/baseline/` path (captured on
the same HEAD; its capture log records gate, demo, fixture, oracle and state-probe runs
with `overall_exit=0`). That baseline is a precondition for a later implementation stage,
not evidence for this decision.

The accepted, implemented and independently verified baseline this design builds on:

- **Minimal Mega Drive slice** ([Mega Drive 68000 slice](../architecture/megadrive-slice.md),
  EPIC-0002). `crates/chronofork-megadrive` implements a 68000 subset, ROM + 64 KiB work
  RAM, a VDP status/control/VRAM port window, a deterministic instruction-quantum
  HBlank/VBlank scheduler with SR-mask-gated delivery and `RTE`, a bounded debug
  instruction trace, and a versioned canonical snapshot. The bus maps cartridge ROM, work
  RAM and the VDP port window; PSG, Z80, mapper and controller windows are deliberately
  unmapped and return typed `UnmappedRead`/`UnmappedWrite` errors. `MegadriveBackend::step`
  ignores its `InputFrame` argument. The slice is not a playable emulator and makes no
  game-compatibility, cycle-accuracy or interrupt-timing claim.
- **Canonical Mega Drive state and snapshot.** `MegadriveBackend` serializes one state body
  (D0–D7, A0–A7, PC, SR, halted flag, tick, 64 KiB RAM, the complete VDP snapshot and the
  complete interrupt-scheduler snapshot) and derives both the payload
  (`CFMD68K2`, payload version 2) and the `CFMDSTATE2` state hash from it. The factory
  hashes the immutable ROM and scheduler configuration into `CFMDCONF2`; a snapshot from
  another ROM, scheduler configuration or payload version is rejected by the existing
  `validate_envelope` predicate on restore/import.
- **M1 oracle differential evidence** ([oracle differential
  evidence](../research/oracle-differential-evidence.md)). Three independently authored
  synthetic ROMs (`rom_alu`, `rom_mem_vdp`, `rom_masked`) match the pinned Exodus oracle's
  projected address streams and selected final state, with passing negative controls and
  the masked-ROM default-quantum gating control. The evidence is PASS 3/3. It compares
  instruction addresses and selected final state only; it is not cycle, interrupt-timing
  or game-compatibility evidence.
- **M3 movie v1/v2, replay, seek, rewind and editing** ([ADR-0004](0004-tas-movie-format.md),
  [ADR-0005](0005-partial-replay-and-branch-comparison.md),
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md),
  [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md),
  [ADR-0008](0008-rerecord-and-branch-editing.md)). Canonical `.cfmovie` v1 and `CFMOVIE2`
  v2 containers embed a verbatim `Snapshot::to_bytes()` checkpoint and per-step
  `(outcome tick, input frame, state hash)` records; replay imports the checkpoint through
  the single `SessionManager::create_session_from_snapshot_bytes` path, applies recorded
  inputs and recomputes state. Ranged replay/seek, anchored seek, branch comparison, the
  bounded in-process rewind cache with `tas studio`, and functional `tas edit` are
  implemented and independently verified **for the deterministic synthetic backend only**.
- **Input model** (`crates/chronofork-core/src/input.rs`). `InputFrame` carries four `u32`
  digital ports (port 0 is the v1 backend-defined button mask), four signed `i16` analog
  axes and a `u64` keyboard bitmap, one frame per step. `new`, `buttons()` and `NEUTRAL`
  preserve the v1 surface; `with_devices`, `ports()`, `axes()` and `keys()` expose the
  schema-2 fields. The synthetic backend consumes non-neutral fields only on `APPLY_INPUT`;
  the Mega Drive slice ignores the whole frame.
- **CLI TAS adapter** (`crates/chronofork-cli/src/{tas,tas_edit,studio,main}.rs`).
  `chronofork tas record|play|verify|compare|studio|edit` print stdout-only `key=value`
  blocks, send diagnostics to stderr as `chronofork: <error>` and exit 0/1. `record`,
  `play`, `verify`, `studio` and `edit` construct a manager over
  `SyntheticBackendFactory::default()`; `compare` never imports a checkpoint and is
  backend-agnostic. At design time `studio` and `edit` had no backend-selection flag, and
  the only ROM-loading CLI path was `chronofork megadrive trace --rom <FILE>`, which read
  the file with `std::fs::read` and constructed `MegadriveFactory::with_scheduler_config`.
- **Deferred items this design addressed.** ADR-0004 §12, ADR-0005 §8, ADR-0006 §13,
  ADR-0007 §9 and ADR-0008 §11 all deferred "real-hardware input TAS: the Mega Drive `step`
  ignores input". At design time the M3 milestone kept "Real-hardware input TAS" unchecked,
  the roadmap's Stage 3 bullet was open, `FR-MD-010` was **Planned** and `FR-TAS-001`
  remained **Partial**; the execution promotion supersedes that state as recorded in
  Status and Evidence.
- **Research context.** [Emulator architecture](../research/emulator-architecture.md)
  records that Exodus models the Mega Drive I/O device `A10000` (version register,
  controller ports, TMSS) and has controller device classes (`MDControl3`/`MDControl6`),
  but exposes no controller input through its MCP surface; its controller behaviour is
  untested. [TAS and debugging](../research/tas-and-debugging.md) records that an immutable
  input schema with explicit digital ports/axes is the reusable pattern and that a movie
  is a recorded per-step input sequence, not a backend-specific event stream. This is
  upstream-derived context, not a ChronoFork decision, and none of it is adopted verbatim.

The controller data-register layout adopted by this ADR is the standard Mega Drive
3-button pad's **TH-high phase** as documented for the original hardware: bits 0–5 are
Up, Down, Left, Right, B and C, active-low (0 = pressed), with bit 6 at the TH-high level
and the A/Start buttons available only in the TH-low phase. That hardware layout is the
basis for the slice contract in §2; this ADR does not adopt a fabricated single-read
union of both phases.

## Problem

How should ChronoFork make the minimal Mega Drive slice a real input-TAS backend without
changing anything already accepted or verified, so that:

1. **Controller reads are exact and small.** A per-step `InputFrame` drives the two
   controller data registers at documented addresses with a documented bit layout; a
   controller read inside an executed instruction observes the current step's input, and
   nothing else changes.
2. **Canonical state is untouched.** The Mega Drive snapshot payload magic, payload
   version, state body, state hash, configuration hash, restore validation, M1 oracle
   evidence, demos, and existing M2/M3 movie bytes and replay results remain exactly as
   they are. Controller input is per-step and is not serialized; a restored machine uses
   the next step's input.
3. **No hidden state survives a step.** The backend wires the input so that bus reads see
   it only during the step, with no persistent latch, no transient field that can leak
   into a snapshot, inspection, or a later step, and no change to the `Backend` trait.
4. **Backend selection is explicit and typed.** The TAS commands that execute a movie can
   select the Mega Drive backend with `--rom <FILE>`; the default synthetic path stays
   byte-identical; a missing, malformed, too-large or wrong ROM and an incompatible
   embedded checkpoint fail with a typed error; `compare` stays backend-agnostic.
5. **Hardware TAS is proved end to end.** An independently authored synthetic 68000 ROM
   reads the controller ports and stores the observed values in work RAM; a movie recorded
   on the Mega Drive backend with `--input`/multi-port masks verifies in a new process,
   survives snapshot/restore/fork/rewind/studio, changes state when controller input
   changes and leaves existing M1 behavior identical under neutral input.
6. **Multi-controller mapping is explicit.** Ports 0 and 1 map to Mega Drive controller
   ports 1 and 2; schema-2 fields the 3-button subset cannot represent are accepted and
   inert, never silently reinterpreted as hardware.
7. **The increment stays honest.** It adds no dependency, `unsafe`, new `#[allow]` or
   `#[ignore]`, claims nothing about TH/6-button/Start multiplexing, real commercial ROMs,
   analog/keyboard hardware, controller timing or cycle accuracy, and M3 completion is
   recorded only after the executed rows and the M3 completion audit pass (§12).

## Alternatives

### Controller model

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Stateless per-step controller reads on the TH-high phase: the bus maps the two data registers and derives each byte from the current `InputFrame`** | No persistent latch or hidden state; input timing is exactly the step quantum; replay/replay-from-checkpoint soundness is structural; the canonical snapshot and state hash cannot change; small, testable emulation surface | Exposes only Up/Down/Left/Right/B/C: A and Start need the TH-low phase and are not readable; a real-hardware read routine that toggles TH cannot run unmodified | **Recommended** |
| B. Model the TH latch/direction register and both TH phases, including A/Start and 6-button identification | Closer to hardware; a real 3-button read routine can work | Adds persistent device state that must be serialized and hashed (changing the payload, state hash and M1 evidence), a write path with direction semantics, 6-button timing/detection rules, and new failure modes; far beyond this increment | Deferred to its own decision (TH/6-button/Start multiplexing) |
| C. Fixed TH-low phase (Up/Down/Left/Right/A/Start) | Exposes A and Start | B and C are primary action buttons and would be unreadable; no less arbitrary than A | Rejected |
| D. Fabricate a single-byte union of both phases (for example A in bit 6) | All 3-button pad buttons readable in one access | The layout is not the hardware data register; a real hardware read routine silently misreads A/B/C; it would be a false hardware claim unless documented as non-hardware, which undermines the purpose | Rejected |
| E. Event queue or scripted input timed by emulated ticks | More realistic scheduling | New event/timing model, new canonical state and determinism questions; ADR-0004/0005/0006/0007 deferred it; no consumer today | Deferred |
| F. Keep ignoring input; hardware TAS stays deferred | Zero risk; no code change | The slice remains unable to consume input, the M3 item stays open, and `tas` cannot record or verify a real hardware input sequence | Rejected for this increment |

The choice between A and B was the owner decision in §11. This ADR adopts A: the
step-quantum model already defines input as per-step, and the increment's goal is honest
input TAS, not a complete controller device.

### Canonical-state treatment of controller input

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Input is per-step and never serialized; a restored machine uses the next step's input** | No payload, state-body, state-hash, payload-version or config change; all M1/M2/M3 evidence stays valid; replay supplies the same per-step inputs in order | A snapshot cannot reproduce "what the controller was doing" mid-step; inspection cannot show a live controller | **Recommended** |
| B. Serialize the current input or TH latch into the snapshot | A restored machine would know the last input | Changes the canonical payload and state body, bumps the payload version, changes every state hash and invalidates M1/M2/M3 evidence and the oracle comparison; also makes input part of checkpoint identity, which is not how a step-quantum machine consumes it | Rejected |
| C. Store input inside the movie checkpoint or a new movie field | One artifact carries it | Movie format change plus a second copy of per-step input; contradicts ADR-0004/0006 where step records already carry the input; invalidates verified movie evidence | Rejected |
| D. Serialize only a TH direction/latch bit | Small canonical addition | Still a payload/state-hash/version change for a state that is not consumed across steps in this model; no benefit without TH modeling | Deferred with option B of the controller model |

### Backend wiring

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. A transient, non-serialized bus field set from `InputFrame` at step entry and cleared on every exit path** | Bus reads see the current step's input; nothing survives the step; no backend-trait or CPU-signature change; snapshot/inspect cannot observe it; restore starts neutral | Adds one private field and a documented set/clear invariant; clone/restore must reset it explicitly | **Recommended** |
| B. A persistent backend or bus field that holds the last input | Simpler read path | Hidden state survives a step and is copied by snapshot/restore/clone; inspection could observe stale input; violates the "no hidden mutable state" rule and the per-step model | Rejected |
| C. Change `M68000::step`/`Backend::step` to pass input to the CPU or bus explicitly | No transient field | Changes the CPU API and every caller for one device; the backend trait already has `step(input)`; needless churn | Rejected |
| D. Global/thread-local input | Trivial access | Forbidden hidden global state; two machines would interfere; determinism would depend on scheduling | Rejected |
| E. Read the controller in the backend before the instruction and patch registers/RAM | No bus change | Wrong layer and wrong semantics; the program must observe the read; it would fabricate state and defeat the TAS contract | Rejected |

### CLI backend selection

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Per-subcommand `--rom <FILE>` on `record`, `play`, `verify`, `studio` and `edit`; absent means the unchanged synthetic factory** | Explicit, local, scriptable; no output-contract change; the default path stays byte-identical; each command decides its own flags | Five parsers gain one flag; `--rom` cannot be used with `--root` in this increment (durable storage is synthetic-only) | **Recommended** |
| B. A global `tas --backend megadrive --rom <FILE>` selector | One place | Changes the top-level dispatch and every subcommand signature; mixes a global mode into commands that do not execute a checkpoint (`compare`); more surface for a first hardware increment | Rejected for this increment |
| C. Auto-detect the backend from the embedded checkpoint's backend id | No repeated flag for play/verify | Still needs a ROM path for the Mega Drive backend; requires exposing a backend-id field or a new probe API; a missing ROM cannot be resolved automatically; hides the configuration | Rejected |
| D. An environment variable or a default ROM path | Zero flags | Implicit host-dependent configuration; violates explicit determinism and the "no host input to hardware" policy; hard to test and audit | Rejected |
| E. A new `tas hardware` subcommand family | Clear separation | Duplicates record/play/verify/studio/edit parsing, output blocks and failure contracts; two spellings for one operation | Rejected |

### Controller-window writes and debugger visibility

| Option | Strengths | Costs and risks | Verdict |
|---|---|---|---|
| **A. Every controller-window write remains a typed `UnmappedWrite`; data-register reads are execution-only, so the side-effect-free debugger path still rejects the window** | Matches the slice's stated principle ("unsupported port accesses return errors, never silent skips"); no TH direction/latch is implied; no fabricated debugger value; no new error variant | A real read routine that writes TH/control fails typed; a debugger cannot inspect a live controller; the synthetic acceptance ROM must read without writing | **Recommended** |
| B. Accept and discard writes to the data/control registers | A real read routine's writes would not fail | Silently ignores direction/TH semantics, contradicting the slice's fail-closed principle; a program could believe it toggled TH and read a wrong phase | Rejected |
| C. Model the control register and TH level | More faithful | Adds persistent device state, payload/hash changes, and the full TH-latch decision; deferred with the controller model | Deferred |
| D. Return a neutral value from debugger reads | Debugger-friendly | Hides that the value is step-scoped and can be misread as a live controller; inspection semantics become ambiguous | Rejected |

## Decision (accepted 2026-11-15)

### 1. Scope and canonical home

This ADR is the canonical home for **real-hardware input TAS on the minimal Mega Drive
slice**: the controller data-register model, its address map and bit layout, the
canonical-state impact, the transient backend wiring, the `--rom` CLI backend selection,
the hardware TAS acceptance design and the deferred list. [ADR-0004](0004-tas-movie-format.md)
remains canonical for the movie bytes, [ADR-0005](0005-partial-replay-and-branch-comparison.md)
for ranged replay and comparison, [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md)
for v2 anchors and schema-2 input, [ADR-0007](0007-in-process-rewind-and-interactive-stepping.md)
for the rewind cache and `tas studio`, and [ADR-0008](0008-rerecord-and-branch-editing.md)
for editing. Architecture, requirements, roadmap and acceptance documents link here
instead of restating the address map, bit layout or CLI surface.

This ADR is **Accepted** (owner-approved 2026-11-15), **implemented and independently
verified** for the deterministic minimal-slice scope (t5 PASS, 2026-11-16). The
implementation followed the [M3 hardware acceptance
plan](../quality/m3-hardware-acceptance.md) rows M3-A45–M3-A53, all
`executed - passed`; the row-level commands and artifacts are recorded in the plan's
execution record and in §12 below. The public names below
(`ControllerInput`, `MegadriveBus::set_step_input`/`clear_step_input`, the read helper,
`read_rom_file`, `--rom`) describe the contract shape; spelling or parameter grouping may
be refined during implementation, but the address map, bit layout, canonical non-change,
wiring invariant, typed rejection boundaries and CLI command set are the contract.

The increment adds no crate, dependency, `unsafe`, `#[allow]`, `#[ignore]`, deleted test
or relaxed lint. The implementation adds only additive help text for the new flag and the
controller limitation; the verified help diff removes the previously false "not
implemented / Mega Drive ignores input" sentence and adds the `--rom` lines and the honest
deferred list.

### 2. Controller model and exact register contract

**Host-facing state.** The controller model is a pure function of the current step's
`InputFrame`:

- Mega Drive controller **port 1** (data register `0x00A10003`) is driven by
  `InputFrame::ports()[0]` (the v1 `buttons()` mask).
- Mega Drive controller **port 2** (data register `0x00A10005`) is driven by
  `InputFrame::ports()[1]`.
- `ports()[2]`, `ports()[3]`, `axes()` and `keys()` are **not** represented by the
  3-button subset. They are accepted by the input schema and recorded by v2 movies, but
  they have **no emulated effect** on this backend. An implementation must not silently
  reinterpret them as other buttons.

**Address map (exact).**

| Address (68000 bus) | Access | Meaning |
|---|---|---|
| `0x00A10003` | byte read | Controller **port 1** data register (3-button TH-high subset) |
| `0x00A10005` | byte read | Controller **port 2** data register (3-button TH-high subset) |
| every other address in `0x00A10000..=0x00A1001F` | any | **Unmapped**: typed `UnmappedRead`/`UnmappedWrite`, including the version register `0x00A10001`, control registers `0x00A10009`/`0x00A1000B`, TX/RX and TH-related addresses |
| `0x00A10003`/`0x00A10005` | byte write | **Unmapped**: typed `UnmappedWrite`; no TH latch/direction is modelled |
| `0x00A10006`/`0x00A10007` | byte read and byte write | **Accepted with no effect and counted** (`MegadriveBus::port3_unmodelled_accesses`): the port 3 (EXT) data register is real hardware (`Data/Modules/Sega Mega Drive 1600.xml:310`/`:311` and `Devices/MD1600IO/A10000.cpp:995-997`, read at `:902-904`) but this slice has no port 3 latch. The write is accepted and the read returns the window's open value; faulting instead turned a legal guest store into a guest address error, which skipped the instruction in a commercial boot (W1X, finding W1V) |

Only **byte** reads are mapped. A word or long access that touches the controller window
(for example `MOVE.W 0x00A10002`) is a typed error because the high byte `0x00A10002` is
unmapped; the slice does not model the 16-bit port alias. Byte writes to the two data
registers are also typed `UnmappedWrite`: the slice has no TH direction or latch state, so
accepting a write and discarding it would silently pretend to implement hardware that does
not exist. This follows the slice's existing rule that unsupported port accesses return
errors and are never silently skipped.

**Bit layout (TH-high phase of the standard 3-button pad).** The byte returned by a data
register read is:

```text
bit 7   0        slice constant; the real register bit is not a defined pin in this subset
bit 6   1        TH high (the read is always the TH-high phase)
bit 5   C        active-low: 0 = pressed
bit 4   B        active-low
bit 3   Right    active-low
bit 2   Left     active-low
bit 1   Down     active-low
bit 0   Up       active-low
```

Therefore `value = 0x40 | (!(mask & 0x3F) & 0x3F)`, where `mask` is the low six bits of
the port field (Up = bit 0, Down = bit 1, Left = bit 2, Right = bit 3, B = bit 4, C = bit 5)
and a set bit means pressed. A neutral port reads `0x7F` (all six inputs released, TH
high, bit 7 zero). Examples: `0x01` (Up) reads `0x7E`; `0x10` (B) reads `0x6F`; `0x20` (C)
reads `0x5F`.

**Buttons represented.** The slice exposes the 3-button pad's TH-high half:
Up, Down, Left, Right, B and C. **A and Start are not readable** in this increment: A is in
the TH-low phase and Start requires the TH-low/6-button phase. A frame whose port mask sets
bit 6 (A) or bit 7 (Start) is accepted and recorded, but those bits have **no emulated
effect** here. This is a deliberate owner decision (§11); it is not a complete 3-button pad
model.

**Fidelity limits that must be stated in the slice doc, help text and evidence:**

- no TH latch, no TH direction register, no TH-low phase, so A/Start are absent;
- no 6-button pad, no X/Y/Z/Mode, no controller-presence or identification protocol;
- no controller serial timing, no 1.5 ms reset window, no response latency;
- no controller-generated interrupt or VDP/controller interaction;
- no analog axes or keyboard hardware; schema-2 axes/keys are inert;
- no port 3, no expansion/peripheral protocol;
- controller reads are per-step, not per-scanline, per-frame or host-arrival timed;
- the debugger cannot inspect a live controller value (see §4).

### 3. Canonical-state impact: none

The increment must not change the canonical Mega Drive snapshot, state hash,
configuration hash, demos, M1 oracle evidence or existing M2/M3 movies. This is a
requirement, not an aspiration:

**Snapshot payload and state body.** `MegadriveBackend`'s canonical state body remains
exactly: D0–D7, A0–A7, PC, SR, halted flag, tick, 64 KiB work RAM, the complete VDP
snapshot and the complete interrupt-scheduler snapshot. The payload magic stays `CFMD68K2`
and the payload version stays **2**. No controller byte, port mask, TH latch or step input
is added to the body, and no new field is added to `StateView`.

**State and configuration hashes.** The `CFMDSTATE2` state hash continues to cover the same
state body, so for the same machine trajectory it is byte-identical before and after this
increment. The `CFMDCONF2` configuration hash continues to cover the ROM and scheduler
configuration only; controller behavior is fixed by the build, not configurable, so no new
configuration field is added. There is no new payload version, no migration and no
reinterpretation of old payloads.

**Input is not serialized.** The current step's `InputFrame` is supplied by the caller of
`Backend::step`. It is not stored in the backend after the step, not part of `snapshot()`
and not part of `inspect()`. A snapshot taken after step N and restored restores the
canonical post-step-N machine; the next `step` uses the next caller-supplied input. This is
exactly the synthetic backend's model and the movie replay model: the movie stores each
step's input in the movie, not in the checkpoint.

**Why this is sound for the step-quantum model.** One step executes exactly one 68000
instruction (one tick). Input is sampled at the step boundary and is visible to every bus
read inside that instruction; after the instruction the canonical state is a function of
the prior canonical state and that instruction's observed reads. There is no pending input
to serialize: the next input is external, supplied by the next step, exactly as the movie's
next recorded frame is. A restored machine therefore cannot hold a stale input, and replay
from the embedded checkpoint with the recorded inputs reproduces the recorded state hashes
because the same per-step frames are supplied in the same order.

**What this does not model.** It does not model a controller being read between
instructions or mid-instruction, a TH latch surviving across instructions, input changing
during an instruction, a controller state that is part of checkpoint identity, or a
snapshot that captures "what the player was holding" independent of the movie. Those are
consequences of the stateless per-step model and are listed in §10.

**Proof obligations for a later implementation.** The acceptance plan requires:

1. a differential serialized-state probe over a neutral-input program, compiled against the
   pre-change tree and the post-change tree, with byte-identical payloads, payload
   lengths, state hashes and configuration hashes;
2. byte-identical existing Mega Drive tests, `megadrive demo` stdout across repeated runs,
   oracle ROM traces/final states and the captured baseline state probes;
3. byte-identical v1/v2 synthetic movie files and transcripts, and unchanged M2 durable
   session fixtures;
4. the full quality gate and both demos.

No implementation may claim "no canonical change" without those executed comparisons.

### 4. Backend wiring: transient, step-scoped, non-serialized

`Backend::step(&mut self, input: InputFrame)` already carries the input. The Mega Drive
implementation consumes it without changing the trait or the CPU API:

- `MegadriveBackend::step` stores the two mapped port masks (or the derived bytes) in a
  **private transient field on `MegadriveBus`** before executing the step, and restores
  the field to neutral on **every** exit path, including delivery, CPU, scheduler, tick and
  status-sync failures. There is exactly one set point and one clear point on the same
  path, so no error return can leave a stale input behind.
- Bus byte reads of `0x00A10003`/`0x00A10005` inside the executed instruction read the
  transient field. No other read path changes.
- `snapshot()` and `inspect()` take `&self` and never read the transient field; the field
  is not serialized, not hashed and not part of configuration. `restore()` explicitly
  resets the replacement bus's transient field to neutral before committing it, so a
  restored machine starts with no input. `Clone` of the bus copies the field, but the
  backend is single-owner and `&mut self` throughout a step, so no clone can occur
  mid-step; the invariant is documented and tested.
- A direct bus read outside a step (test/tooling) observes the neutral idle byte, never the
  previous step's input. The side-effect-free debugger path (`peek8` via `read_memory`)
  does **not** map the controller window: it keeps returning the existing typed error,
  because a step-scoped value is not a machine register and must not be presented as one.
- There is **no** new `Backend` trait method, no capability bit, no new inspection
  pseudo-register and no new public `Session`/`SessionManager` API. The generic backend
  model is unchanged. Adding a controller capability field was rejected for this increment
  because it would change the shared capability schema and the `backend describe` output
  while the model is still a subset; a later capability-model decision can add it with the
  matching evidence.

Rejected wiring alternatives are in the alternatives table: a persistent field, a CPU
signature change, global state, and pre-instruction register patching.

### 5. CLI `--rom` backend selection

The implemented surface is deliberately small and per-subcommand:

```text
chronofork tas record  ... [--rom <FILE>]
chronofork tas play    ... [--rom <FILE>]
chronofork tas verify  ... [--rom <FILE>]
chronofork tas studio  ... [--rom <FILE>]
chronofork tas edit    ... [--rom <FILE>]
chronofork tas compare ...                  # no --rom; backend-agnostic
```

- **Factory selection.** With `--rom <FILE>`, the command reads the file bounded and
  constructs `Arc<MegadriveFactory>` through the existing `MegadriveFactory::new` (or the
  existing scheduler-config constructor with the default NTSC configuration); without it,
  the command uses the existing `SyntheticBackendFactory::default()` path. The manager is
  the existing `SessionManager::new(Arc<dyn BackendFactory>)`; no new manager, import path
  or compatibility predicate is added.
- **Default path is byte-identical.** No `--rom` means the same factory, manager, inputs,
  serialization and output blocks as today. Existing synthetic movies re-encode to the
  same bytes, existing transcripts are unchanged and the help text changes only by the
  added flag documentation.
- **Bounded ROM read.** The CLI opens the path read-only, requires a regular file, reads at
  most `ROM_END + 1` bytes (4 MiB cartridge window) and rejects a larger file with a typed
  CLI error before constructing the factory. `MegadriveFactory` still rejects an image
  shorter than the 8 reset-vector bytes or larger than the window with its existing
  `BackendError::InvalidConfiguration`. A missing/unreadable path, a directory and an
  oversized file are typed CLI failures with empty stdout and exit 1.
- **Wrong or incompatible ROM is typed.** `record --rom` constructs a fresh Mega Drive
  session; a ROM whose reset vectors are unusable fails with the existing typed backend
  error when the session executes (`Execution` for an unmapped fetch; the factory's
  `InvalidConfiguration` covers an undersized image), never a panic. `play`, `verify`,
  `studio` and `edit` import the movie's embedded checkpoint through the existing
  `SessionManager::create_session_from_snapshot_bytes`; if the embedded checkpoint's backend
  id, backend version, payload version or configuration hash does not match the selected
  factory, the existing `validate_envelope` path rejects it as
  `SessionError::IncompatibleSnapshot`. That covers a synthetic movie with `--rom`, a Mega
  Drive movie without `--rom`, a Mega Drive movie with a different ROM (different
  `CFMDCONF2`), and a different scheduler configuration. The CLI maps the typed session
  error to stderr + exit 1; it never retries with another backend.
- **`--rom` with durable storage is rejected.** `record --root ... --rom` is a typed CLI
  error in this increment: M2 durable storage is a synthetic-backend surface, no Mega Drive
  checkpoint can be published there by the existing CLI, and allowing the combination would
  only produce a confusing import incompatibility. A future durable-hardware-session
  decision can extend this deliberately.
- **`--rom` on `compare` is an unknown-option error.** `compare` reads movie records only,
  never imports a checkpoint and never executes a backend; it stays backend-agnostic.
  `--rom` is also rejected by unrelated subcommands and by `tas studio --help` unless it is
  a documented flag of that command.
- **No ROM mutation and no ROM embedding.** The ROM file is opened read-only and never
  written, renamed, truncated or deleted. A movie does not embed the ROM: the embedded
  checkpoint carries the configuration hash, so replaying a Mega Drive movie requires the
  same ROM file. The CLI never copies a ROM into a movie, a storage root or a temporary
  file, and no ROM bytes appear in stdout, stderr or evidence text.
- **Additive changes to ADR-0007/ADR-0008 CLI surfaces.** `studio` and `edit` gain a
  backend-selection flag. Their command protocols, blocks, cache/edit semantics and
  failure contracts are unchanged; ADR-0007 §6's "studio has no backend-selection flag"
  and ADR-0008 §6/§7's synthetic-factory statement are extended only for this flag, not
  superseded. The owner approved that additive change (§11).

The full flag list, output blocks and process tests are in the executed
[M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md) §5–§6.

### 6. Hardware TAS acceptance design

**Independently authored synthetic ROM.** Acceptance uses one deterministic synthetic 68000
program generated in code (`controller_rom()` beside the existing
`demo_rom()`/`interrupt_rom()`), never a commercial ROM, BIOS or copyrighted asset. The
program:

1. sets up no controller registers (the slice has no TH direction);
2. in a loop, reads `0x00A10003` (port 1) and `0x00A10005` (port 2) with byte absolute-long
   loads;
3. accumulates each observed byte into a separate data register with `ADD.B` and stores the
   accumulators at fixed work-RAM addresses (for example `0x00FF0000` and `0x00FF0001`) so
   the observed controller values are part of canonical RAM/state;
4. self-loops with a short branch, so a bounded instruction budget or movie step count ends
   the run deterministically;
5. writes no `SR`, no controller register and no VDP register, and sets no interrupt
   handler; at the reset mask 7 the VBlank/HBlank requests stay masked, so the measured
   window is interrupt-free in the same sense as the existing `rom_masked` control;
6. uses only instruction forms already implemented and tested by the slice (documented
   subset), fits inside the 4 MiB cartridge window and is deterministic (no host time,
   entropy or file input).

The ROM is authored against the slice contract of §2. It does not claim to be a
real-hardware read routine, because it reads only the TH-high phase and performs no TH
toggling. Its purpose is to make controller input observable in canonical state, not to
validate a commercial controller protocol.

**Record, play and verify.** With the ROM file:

- `tas record --rom <rom> --movie <file> --steps <N> --input <mask>` records a fixed
  per-step port-1 mask on the Mega Drive backend; `--ports P1,P2,P3` records schema-2
  frames for two-controller input. The output block is the existing `record=movie` block;
  `movie_version`/`input_schema` follow the existing v1/v2 selection rule.
- `tas verify --rom <rom> --movie <file>` in a **new process** imports the embedded Mega
  Drive checkpoint, applies every recorded frame, recomputes every tick and state hash and
  prints `verified=1` only on success.
- `tas play --rom <rom> --movie <file>` reproduces the recorded end tick and final state
  hash. Divergence uses the existing typed first-divergence report.
- Two recordings of the same ROM and steps with different port masks produce different
  per-step/final state hashes and different `movie_sha256` values; `tas compare` reports
  the first divergent index/tick/hash without needing `--rom`. The same input and a fresh
  manager produce byte-identical movies.

**Snapshot, restore, fork, rewind and studio.** The acceptance must show, at the lowest
useful layer and through the CLI where applicable:

- snapshot → restore → continue with the next input reproduces the uninterrupted run's tick
  and state hash; the restored machine uses the next step's input because none was
  serialized;
- two forks from one checkpoint stepped with different next inputs (for example port 1
  pressed versus neutral) produce different state hashes, and the source session is
  unchanged;
- `tas studio --rom <rom> --movie <file>` opens the hardware movie, steps/rewinds through
  `next`/`back`/`goto`/`state` with the bounded cache, verifies recorded expectations and
  writes no file; its blocks match a fresh replay and the movie SHA-256 is unchanged;
- a `tas verify` after the studio run still passes.

**Neutral input leaves existing M1 behavior identical.** The acceptance must show that:

- the three oracle ROMs do not access the controller window (the program sources/bytes are
  audited), so their instruction traces, final states and oracle comparison remain identical;
- existing Mega Drive unit/integration tests, the `megadrive demo` stdout and the captured
  neutral-input state probes are byte-identical to the pre-change baseline;
- the differential serialized-state probe (neutral input) is byte-identical in payload,
  payload length, state hash and configuration hash;
- existing synthetic and M2/M3 fixtures are unchanged.

**What is proved and what is not.** A PASS proves that per-step controller reads are wired,
observable, deterministic and TAS-replayable on the deterministic slice, and that canonical
state/evidence is unchanged for neutral input. It does **not** prove real controller
protocol fidelity, A/Start/6-button behavior, controller timing, commercial-ROM
compatibility, rendering/audio/game behavior or cycle accuracy.

### 7. Multi-controller and schema-2 field mapping

| Mega Drive port | Data register | `InputFrame` field | Represented here |
|---|---|---|---|
| Controller 1 | `0x00A10003` | `ports()[0]` (v1 `buttons()`), low 6 bits | Up/Down/Left/Right/B/C; A (bit 6) inert, Start (bit 7) inert |
| Controller 2 | `0x00A10005` | `ports()[1]`, low 6 bits | Up/Down/Left/Right/B/C; A/Start inert |
| — | — | `ports()[2]`, `ports()[3]` | recorded but inert (no Mega Drive port 3/4 model) |
| — | — | `axes()[0..4]` | recorded but inert (no analog hardware) |
| — | — | `keys()` | recorded but inert (no keyboard hardware) |

The 3-button subset is the same for both ports. A two-controller test must show independent
values: a mask on port 1 must change only the port-1 accumulator byte, and a mask on port 2
only the port-2 accumulator byte, with distinct state hashes when either changes. The
service layer and `InputFrame` schema are unchanged: v1 movies reconstruct neutral extra
fields; v2 movies carry the full frame; unsupported fields have no effect and must never be
described as hardware support.

This mapping is a backend contract, not a ChronoFork-wide controller layout. A different
backend may assign the same frame fields differently, and the generic layers must not
encode Mega Drive assumptions.

### 8. Limits and typed errors

| Limit / error | Contract | Owner |
|---|---|---|
| ROM size | At least 8 reset-vector bytes; at most the 4 MiB cartridge window (`ROM_END`) | Existing `MegadriveFactory` (`BackendError::InvalidConfiguration`); CLI bounded read rejects an oversized file before construction |
| ROM file | Must exist, be a regular file and be readable; never written | CLI `CliError`; read-only open |
| ROM window read | Only byte reads at `0x00A10003`/`0x00A10005` are mapped; every other controller-window read is `UnmappedRead` → the existing `BackendError::Execution` mapping (debugger reads are rejected as unmapped) | `MegadriveBus` |
| Controller-window write | Any write, including the data registers, is `UnmappedWrite`; no silent no-op | `MegadriveBus` |
| Wrong backend / configuration | Embedded checkpoint import through the existing path; mismatch is `SessionError::IncompatibleSnapshot` preserving the existing typed source | `SessionManager` / `validate_envelope` |
| `--rom` misuse | `--rom` on `compare` or an unrelated command, missing value, duplicate value, or `--rom` with `--root`/`--session`/`--snapshot` is a typed CLI failure before any execution | CLI parsers |
| Inputs not represented | `ports()[2..3]`, axes and keys are inert on this backend; no error, no reinterpretation, documented in help/docs | Backend contract |
| Resource bounds | One bounded ROM read; no allocation sized from an unvalidated file length; snapshot/anchor/movie limits unchanged | CLI + existing writers |
| Dependencies / lints | No new dependency, no `unsafe`, no new `#[allow]`/`#[ignore]`, no deleted or weakened test | Whole increment |

There is **no new public error enum** and no second compatibility predicate. The controller
window reuses the existing `BusError` variants and the existing backend mapping; backend
selection reuses the existing `SessionError`/`CliError` categories. No error is stringly
flattened from a typed source. An implementation must not add a "controller not modelled"
variant merely for wording; the existing unmapped/incompatible categories are the contract.

RAM, ROM, VDP and scheduler state, movie/checkpoint bounds, step counts, anchor budgets and
configuration limits are unchanged. A ROM is never mutated, and no ROM or copyrighted
asset is embedded, copied into a movie or committed.

### 9. Acceptance plan

The executable acceptance plan is the [M3 hardware
acceptance](../quality/m3-hardware-acceptance.md), rows **M3-A45–M3-A53**. Every row is
`executed - passed` (independent t5 verification, 2026-11-16); row-level commands and
artifacts are in the plan's execution record and §12:

| Row | Coverage | Status |
|---|---|---|
| M3-A45 | Controller read semantics, exact address map/bit layout, two ports, writes/debugger rejection, and canonical-state non-change under neutral input | executed - passed |
| M3-A46 | Hardware `record`/`play`/`verify` end to end in new processes, input-dependent state, divergence/compare controls | executed - passed |
| M3-A47 | Hardware movie snapshot/restore/fork/rewind/`studio` behavior and non-mutation | executed - passed |
| M3-A48 | Two-controller input mapping and inert schema-2 fields | executed - passed |
| M3-A49 | CLI `--rom` contract: accepted commands, bounded read, default-path byte equality, missing/wrong ROM and incompatible-checkpoint errors, no ROM mutation | executed - passed |
| M3-A50 | Determinism, debug/release equality, bounds, typed-error and structural/suppression audit | executed - passed |
| M3-A51 | M1/M2/M3 non-regression: existing Mega Drive tests, oracle ROMs, demos, v1/v2 movies, M2 storage, full gate | executed - passed |
| M3-A52 | Oracle differential harness gate: PASS 3/3 with identical digests and final states, plus the masked gating control | executed - passed |
| M3-A53 | Scope honesty: docs/help match evidence, deferred list, no overclaim (the M3 completion record follows the executed audit) | executed - passed |

A row passes only on executed commands with recorded exit codes, tree state and artifacts;
this ADR, a code review, a demo or a single happy-path run is not acceptance evidence.
### 10. Deferred work and stated limitations

**Delivered and verified for this increment (deterministic minimal-slice scope):**

- the stateless per-step controller data-register model for Mega Drive ports 1 and 2;
- the transient, non-serialized bus wiring with no backend-trait change;
- the `--rom` selection on `record`, `play`, `verify`, `studio` and `edit`;
- the independently authored synthetic acceptance ROM and end-to-end hardware
  record/play/verify/snapshot/restore/fork/rewind/studio acceptance;
- executable tests and honest documentation of what remains.

**Deferred, not implemented and not to be claimed:**

- TH latch and direction register, TH-low phase, and therefore **A and Start** on the
  3-button pad;
- 6-button pad support (X/Y/Z/Mode), controller identification/presence protocols and the
  1.5 ms reset window;
- real controller timing, per-scanline/per-frame input, input events scheduled by emulated
  time, and controller-generated interrupts;
- analog axes, keyboard hardware, port 3, expansion/peripheral protocols and multiplayer
  adapters;
- real commercial ROMs, ROM headers/region/TMSS, cartridge mappers and game compatibility;
- cycle accuracy, bus-cycle/prefetch timing, rendering, audio, Z80 and DMA;
- import/export adapters for other TAS formats (BizHawk BK2 and others);
- a controller capability bit or new inspection pseudo-registers;
- durable Mega Drive sessions in M2 storage and `record --root --rom`;
- cross-backend canonicalization, cross-platform runtime determinism and any
  hardware-accuracy claim.

**M3 completion audit (executed 2026-11-16).** The independent t5 verification executed
the completion audit and confirmed with evidence: the accepted status and owner approval of
this ADR; every M3-A45–A53 row `executed - passed` against the recorded tree and artifacts;
the unchanged canonical snapshot/state hash/config hash and unchanged M1 oracle evidence;
unchanged demos, M2/M3 fixtures and movie bytes; the full quality gate; and that the
deferred list above is still honestly unimplemented. The audit covered the five remaining
M3 milestone items — anchors/seek, rewind/interactive stepping, rerecord/branch editing,
multi-device input and real-hardware input TAS — each with executed evidence
(`SUMMARY.txt` §9). **M3 is complete for the documented deterministic minimal-slice scope**;
the t6 promotion records it and the t7 promotion audit returned PASS on 2026-11-16.
Import/export adapters remain documented-only and are not claimed.

### 11. Owner decisions (approved 2026-11-15)

The owner explicitly approved the following decisions as proposed on 2026-11-15; the
approval statement and timestamp are recorded in the header and Evidence.

1. **Controller model:** stateless per-step reads of the two data registers, the exact
   address map (`0x00A10003`/`0x00A10005` byte reads only), the TH-high active-low bit
   layout (`0x40 | ((!mask & 0x3F) & 0x3F)`, neutral `0x7F`), and the consequence that
   **A and Start are not readable** in this increment.
2. **Writes and TH direction:** every controller-window write remains a typed
   `UnmappedWrite`; no TH latch/direction, no silent no-op writes, and debugger reads of
   the controller window remain rejected.
3. **Canonical-state stance:** controller input is per-step, not serialized, and a restored
   machine uses the next step's input; no payload/version/state-body/state-hash/config
   change and no new canonical field.
4. **Backend wiring:** one transient step-scoped `MegadriveBus` field set/cleared on every
   step path; no `Backend` trait change, no capability bit, no new inspection surface.
5. **CLI `--rom` surface:** `record`/`play`/`verify`/`studio`/`edit` accept `--rom <FILE>`;
   `compare` stays backend-agnostic; `record --rom --root` is rejected; the default
   synthetic path stays byte-identical.
6. **Hardware acceptance fixture:** one independently authored synthetic 68000 ROM built in
   code, reading both ports and storing observed values in work RAM, with no commercial
   ROM, plus the snapshot/restore/fork/rewind/studio and M1 non-regression acceptance
   design.
7. **Scope and deferred list:** §10 stays deferred; this increment does not by itself
   complete M3 until the executed rows and the completion audit pass (now recorded in §12),
   and it does not authorize a commit or push.

The approval authorizes implementation only under the approved staged plan and this
contract. If a different controller model (for example a TH latch) is later preferred, that
is a different decision with its own payload/state-hash and evidence consequences; this ADR
must then be amended or superseded, not silently reinterpreted.

### 12. Verification executed (2026-11-16)

The independent t5 verifier executed the companion acceptance rows and the M3 completion
audit on HEAD `85af5238fabf204bbb885df3823c604212edd948` (branch `agent/m3-complete`) plus
the increment working-tree paths, with `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**
(2026-11-16T03:04:06Z–03:27Z). The evidence root
`.research/m3-hardware/verification/` contains 1057 artifacts bound by
`MANIFEST.sha256` (`sha256sum -c` exit 0); the verifier's only tracked edit is the dated
PASS block in [quality gates](../quality/quality-gates.md). No commit, push, tag or merge
was made.

**Rows.** Every M3-A45–M3-A53 row is `executed - passed` with an executed command, exit
code and artifact under `rows/`:

| Row | Verified evidence |
|---|---|
| M3-A45 | bus 13, backend 38 and controller 6 tests passed; differential probe payload/envelope/meta byte-identical pre/post; 23/23 per-step state hashes identical for neutral vs extreme frames; pre-change snapshot restores post-change and vice versa |
| M3-A46 | CLI hardware target 13 passed in debug and release; hardware record/play/verify transcripts; neutral final hash `6d1677d5…` vs Up+B `2d97debc…`; compare divergence index 4; observed RAM values match `0x40` OR `(!mask & 0x3F)`; wrong backend/ROM errors typed; ROM read-only and not embedded |
| M3-A47 | controller 6 and CLI 13 tests passed; snapshot/restore/fork with the next frame; scripted studio 11 blocks, 2 cache evictions, goto-2 state equal to ranged verify; studio writes nothing; wrong-factory failures typed |
| M3-A48 | per-bit/two-controller `[7e,5f,7e,5f]` vs swapped; v2 anchors=3, port-2 change diverges at index 8; A/Start and ports[2..3]/axes/keys inert; port 3 unmapped; v1/v2 schema preserved |
| M3-A49 | 18 typed-error transcripts (missing/dir/short/4 MiB+/unreadable/wrong ROM, wrong backend, `--root`/`--session`, `compare --rom`, missing/duplicate `--rom`); default path 59/59 transcripts and 19/19 movies byte-identical; ROM read-only; help additive and honest |
| M3-A50 | debug/release movie bytes identical (`3e11e762…`); bounds and typed errors observed; one structural controller/manager path; no suppression/dependency additions; AS-14 counts recorded |
| M3-A51 | full gate below; 3 cargo + 3 direct-binary demo runs per demo byte-identical to baseline; 59/59 fixtures and 19/19 movies identical; two-root session canonical bytes and state hash `a4c253d2…` identical; state probes identical; M1 oracle green |
| M3-A52 | `oracle-diff.py --skip-build` exit 0, status=pass, 3/3 traces, 3/3 final states, 3/3 negative controls, 3/3 scheduler controls, masked gating 1/1; per-ROM JSON fields identical to baseline; ROMs contain no controller-window access |
| M3-A53 | honesty/no-overclaim audit passed (no ROM/BIOS, deferred list intact); the t6 durable-doc promotion completed its pending half |

**Canonical Mega Drive state, demos and M1 non-change.** Three independent executed lines
prove the non-change: the differential probe built against `git archive 85af523` and the
working tree produced byte-identical payload, envelope and metadata for a neutral
non-controller ROM (payload SHA-256 `db6e61db…`, length 131255, state hash `87c57711…`,
configuration hash `a8b7ffb0…`); the golden snapshot/config/state assertions in
`crates/chronofork-megadrive/tests/snapshot_properties.rs` (10 passed) and the unchanged
`CFMD68K2` payload version, state body and `CFMDSTATE2`/`CFMDCONF2` hold; and the two demos
are byte-identical to the pre-change baseline (synthetic 986 B `9b4dd564…`, Mega Drive
1582 B `912a5bb6…`). The three oracle state/trace probes and the oracle JSON are identical
to baseline, the oracle harness passes 3/3 with all controls, and the oracle ROMs contain
zero controller-window accesses.

**Gate and regression.** `cargo fmt --all -- --check`, `cargo check --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `./scripts/check.sh` and
`git diff --check` all exited 0. `cargo test --workspace` = 532 passed / 0 failed / 0
ignored over 39 targets (baseline 502/37); the two new targets are
`chronofork-megadrive/tests/controller.rs` (6) and `chronofork-cli/tests/tas_hardware.rs`
(13), and every baseline target is present with `passed >= baseline`. No `#[allow]`,
`#[ignore]`, `unsafe`, deleted test, dependency or manifest change was added.

**Scope of the claim.** The hardware claim is limited to the independently authored
synthetic 68000 `controller_rom()` and the TH-high 3-button subset (Up/Down/Left/Right/B/C,
active-low) on controller ports 1–2 of the deterministic minimal Mega Drive slice on Linux.
A and Start, TH/6-button/Start multiplexing, controller timing, analog/keyboard hardware,
port 3, real commercial ROMs, rendering/audio/Z80/DMA, cycle accuracy and import/export
adapters (BizHawk BK2 and others) remain unimplemented and are not claimed. No O(1) or
persistent seek/cache claim is changed.

**M3 completion.** The audit in `SUMMARY.txt` §9 covers the five remaining M3 milestone
items — anchors/seek, in-process rewind/interactive stepping, rerecord/branch editing,
multi-device input and real-hardware input TAS — each with executed evidence. **M3 is
complete for the documented deterministic minimal-slice scope**; this ADR's t6 promotion
records it, and the t7 promotion audit returned PASS on 2026-11-16. No commit or push is
claimed.

## Rationale

- **Per-step stateless input matches the step-quantum machine.** The backend already
  executes exactly one instruction per step and the movie already stores one frame per
  step. A stateless read makes the emulated input a pure function of that frame, so replay
  from a checkpoint is sound without serializing input.
- **Not serializing input is the only way to preserve every canonical byte and hash.** The
  Mega Drive payload, state hash and M1 evidence are already accepted and independently
  verified. A latch or serialized input would change all of them for a capability that the
  step model does not need.
- **A transient field is the smallest correct wiring.** The bus is the device the CPU reads;
  setting the value only for the executed instruction and clearing it on every path gives
  the read semantics without a trait change, a CPU signature change or hidden state that
  outlives the step.
- **The TH-high subset is the honest minimum for a stateless model.** A/Start require the
  TH-low phase. Fabricating a union byte or silently accepting TH writes would produce a
  hardware claim the slice cannot back; the decision instead exposes six buttons, states
  the loss and defers the latch.
- **Fail-closed writes and debugger reads follow the slice's existing rule.** Unsupported
  port accesses already return typed errors and are never silently skipped. Keeping that
  rule for controller writes and for step-scoped debugger reads avoids a second, softer
  contract.
- **Per-subcommand `--rom` keeps the existing TAS surfaces intact.** The commands already
  share one manager/import path; selecting a factory per command adds one flag without a
  global mode, while `compare` stays a pure record diff.
- **A synthetic ROM makes the claim testable without ROM licensing.** Independently
  authored code that reads the ports and stores the values in RAM gives canonical state
  that changes with input, so record/play/verify and snapshot/fork tests can prove the
  wiring without a commercial ROM or a hardware oracle.

## Consequences

Positive:

- The Mega Drive slice becomes an honest input-TAS backend for the documented subset.
- Canonical snapshots, state hashes, configuration hashes, demos, M1 oracle evidence and
  existing M2/M3 movies stay unchanged.
- Replay, seek, rewind, studio and edit work on a hardware movie through the existing
  session/movie paths, with no new format or replay implementation.
- Backend selection is explicit and typed; the default synthetic path is untouched.
- Controller limitations are stated in one canonical decision instead of being scattered
  or implied.

Negative / costs:

- The slice cannot read A or Start, cannot run a real hardware read routine unmodified,
  and cannot identify a 6-button pad.
- Controller input is not part of checkpoint identity; a snapshot alone does not say what
  was being held, and inspection cannot show a live controller.
- The CLI gains one flag on five subcommands, and `record --rom --root` is unsupported in
  this increment.
- The synthetic acceptance ROM is a fixture with limited coverage, not a hardware
  compatibility test.

Risks / limitations:

- The chosen bit layout and address map are the slice contract; a reviewer could reasonably
  prefer a TH-latch model. That is why owner approval is required before implementation.
- A movie recorded on the Mega Drive backend requires the exact ROM file to replay; the ROM
  is not embedded and must not be committed if it were ever non-synthetic.
- The acceptance claim is limited to the deterministic slice on Linux; no controller
  protocol fidelity, real-commercial-ROM compatibility, timing, rendering, audio, Z80 or
  cycle accuracy is claimed.
- This ADR is **Accepted** (owner-approved 2026-11-15): acceptance authorizes implementation
  under a staged plan, not an assertion that any implementation, test or verification
  exists.

## Status

**Accepted, implemented and independently verified.** The project owner approved this
decision as proposed on 2026-11-15 at 2026-11-15T23:38:06-03:00 (America/Fortaleza),
2026-11-16T02:38:06Z; the approval statement and approved decisions are recorded in the
header, §11 and Evidence. The increment was implemented and independently verified for the
deterministic minimal Mega Drive slice on Linux: the t5 verifier executed every
M3-A45–M3-A53 row and the M3 completion audit and returned **PASS** (2026-11-16; §12). The
canonical Mega Drive snapshot, state body, state hash, configuration hash, demos, M1 oracle
evidence and existing M2/M3 movies are unchanged and byte-identical to the pre-change
baseline. **M3 is complete for the documented deterministic minimal-slice scope**, recorded
by this t6 promotion; the t7 promotion audit returned PASS on 2026-11-16. Limits remain
stated in §10: A/Start, TH/6-button/Start multiplexing, controller timing,
analog/keyboard hardware, port 3, real commercial ROMs, cycle accuracy and import/export
adapters are not implemented or claimed. No commit or push was made.

## Evidence

- **Owner approval (2026-11-15T23:38:06-03:00):** the project owner explicitly approved
  ADR-0009 as proposed with the statement "Aprovar ADR-0009 como proposto (Recomendado)" at
  2026-11-15T23:38:06-03:00 (America/Fortaleza), 2026-11-16T02:38:06Z. The approval covers
  the proposed contract as recorded here: controller address map and bit layout, the
  A/Start limitation, fail-closed controller-window writes and debugger/`peek8` rejection,
  non-serialized input and canonical non-change, transient bus wiring with no `Backend`
  trait/capability change, the `--rom` surface and typed errors, the synthetic controller-
  ROM acceptance, the deferred list and no M3-completion claim. It is a decision approval,
  not an implementation or verification claim, and it does not authorize a commit or push.
- **Tree under test:** `agent/m3-complete` at HEAD
  `85af5238fabf204bbb885df3823c604212edd948` plus the 19 increment working-tree paths
  (recorded in `.research/m3-hardware/verification/tree/`). The pre-change baseline is
  `.research/m3-hardware/baseline/` (same HEAD, clean tree; capture log `overall_exit=0`,
  manifest verified before and after verification).
- **Independent verification (t5 PASS, 2026-11-16):** 1057 artifacts under
  `.research/m3-hardware/verification/` bound by `MANIFEST.sha256` (`sha256sum -c` exit 0);
  every M3-A45–A53 row `executed - passed` with command, exit code and artifact under
  `rows/`; full gate all exit 0 (39 targets, 532 passed, 0 failed, 0 ignored; baseline
  37/502/0/0); demos, 59/59 fixture transcripts, 19/19 movies, three oracle state/trace
  probes, the oracle JSON, the M1 harness (3/3 traces, 3/3 final states, 3/3 negative
  controls, 3/3 scheduler controls) and the differential payload/envelope/meta probe all
  byte-identical to baseline. The dated PASS block is in
  [quality gates](../quality/quality-gates.md). No commit, push, tag or merge was made.
- **Implemented and verified paths (t5):**
  `crates/chronofork-megadrive/src/backend.rs` (`MegadriveBackend::step` consumes the
  per-step `InputFrame` and sets/clears the transient bus input; `controller_rom()`; the
  unchanged `state_body`/`state_hash`/`snapshot`/`restore`; `PAYLOAD_VERSION = 2`;
  `CFMD68K2`/`CFMDSTATE2`/`CFMDCONF2`); `crates/chronofork-megadrive/src/bus.rs` (byte
  reads of `0x00A10003`/`0x00A10005` via `0x40 | ((!mask & 0x3F) & 0x3F)`, transient
  neutral field, typed unmapped writes and `peek8`); `crates/chronofork-megadrive/src/lib.rs`
  (exports `controller_rom` and controller constants);
  `crates/chronofork-megadrive/tests/controller.rs` (6 tests) and
  `crates/chronofork-megadrive/tests/snapshot_properties.rs` (10); CLI
  `crates/chronofork-cli/src/{tas,studio,tas_edit,main}.rs` (`--rom`, bounded
  `read_rom_file`, factory selection, unchanged default synthetic path) and
  `crates/chronofork-cli/tests/tas_hardware.rs` (13).
- **Input and backend contracts:** `crates/chronofork-core/src/input.rs` (`InputFrame`,
  ports/axes/keys, `new`/`with_devices`/`buttons`/`ports`/`axes`/`keys`/`NEUTRAL`);
  `crates/chronofork-backend-api/src/traits.rs` (`Backend::step(InputFrame)`,
  `BackendFactory`); `crates/chronofork-backend-api/src/snapshot.rs`
  (`validate_envelope`).
- **Session and movie paths this design reuses:** `crates/chronofork-sessions/src/manager.rs`
  (`SessionManager::new`, `create_session_from_snapshot_bytes`,
  `create_session_from_stored_snapshot`); `crates/chronofork-sessions/src/movie.rs`;
  `crates/chronofork-sessions/src/replay.rs`;
  `crates/chronofork-sessions/src/player.rs`; `crates/chronofork-sessions/src/edit.rs`.
- **CLI paths this design extends:** `crates/chronofork-cli/src/tas.rs`
  (`parse_record_options`, `tas_record`, `parse_replay_options`, `replay_movie`,
  `read_movie_file`, `write_movie_file`, help); `crates/chronofork-cli/src/studio.rs`
  (options, `SyntheticBackendFactory`, `Player::open`); `crates/chronofork-cli/src/tas_edit.rs`
  (`synthetic_session_manager`); `crates/chronofork-cli/src/main.rs`
  (`synthetic_session_manager`, `megadrive trace --rom` read/factory path).
- **M1 evidence and harness:** [oracle differential
  evidence](../research/oracle-differential-evidence.md); `scripts/oracle-diff.py`
  (`--skip-build`, pinned Exodus oracle under `.research/upstream/exodus-headless` and
  `.research/oracle/build` present in this checkout at design time); the three synthetic
  ROMs' projected address streams, final states and negative controls.
- **Existing acceptance records (regression preconditions, not this increment's
  acceptance):** M3-A1–A12 in the [M3 acceptance
  plan](../quality/m3-tas-replay-acceptance.md); M3-A13–A17 in the [increment-2
  acceptance plan](../quality/m3-partial-replay-acceptance.md); M3-A18–A27 in the
  [format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md); M3-A28–A34 in the
  [M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md); M3-A35–A44 in the
  [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md); the dated
  [quality gates](../quality/quality-gates.md) blocks.
- **Requirements/milestone state at promotion:** `FR-MD-010` is **Implemented (3-button
  TH-high subset; ports 1–2)** and `FR-TAS-001` is **Implemented** with import/export
  adapters separately documented-only; `FR-EXEC-002` covers the verified hardware
  controller reads. The M3 "Real-hardware input TAS" item and the M3 milestone are complete
  for the documented deterministic minimal-slice scope in
  [milestones](../roadmap/milestones.md) and [roadmap](../roadmap/roadmap.md); the
  subsequent t7 promotion audit returned PASS on 2026-11-16.
- **M3 completion audit (t5, 2026-11-16):** the five remaining M3 milestone items —
  anchors/seek (movie_v2 8, replay 23), rewind/interactive stepping (player 16, studio 11),
  rerecord/branch editing (edit 22, tas_edit 19), multi-device input (tas 21, movie_v2 8)
  and real-hardware input TAS (controller 6, CLI hardware 13, rows A45–A52) — each have
  executed evidence; import/export adapters remain documented-only. See `SUMMARY.txt` §9.
- **Evidence scope:** the t5 artifacts are the executable evidence for the scope claimed
  here. They do not support any deferred item in §10, any commercial-ROM or game-
  compatibility claim, any controller-timing/TH/6-button claim, or any cycle-accuracy
  claim.

## Change control

- This ADR is **Accepted, implemented and independently verified** (owner-approved
  2026-11-15; t5 PASS 2026-11-16). A change to the address map, bit layout, mapped access
  width, write behavior, canonical non-change requirement, transient-wiring invariant, CLI
  command set or acceptance structure requires an explicit amendment and re-verification.
  It must not be made by quietly renaming a test, mapping an extra register or loosening an
  assertion.
- Any implementation that changes a snapshot payload byte, payload version, state hash,
  configuration hash, demo byte, oracle artifact, existing movie byte or existing CLI
  output is a canonical behavior change under [ADR-0004](0004-tas-movie-format.md)/
  [ADR-0006](0006-movie-format-v2-anchors-and-multi-device-input.md) change control: stop,
  re-baseline, version it if canonical, and get a new owner decision.
- If the owner later chooses a TH-latch or 6-button model instead, this ADR is amended or
  superseded before implementation; the stateless model must not be partially implemented
  and then described as the approved one.
- **2026-11-15 (owner approval).** Status changed from Proposed to Accepted on the owner's
  statement "Aprovar ADR-0009 como proposto (Recomendado)" at
  2026-11-15T23:38:06-03:00 (America/Fortaleza), 2026-11-16T02:38:06Z. No contract address,
  bit layout, limit, command, typed boundary or deferred item changed; no implementation or
  verification is claimed.
- **2026-11-16 (execution promotion, t6).** The independent t5 verification PASS promoted
  the implementation status, callout, §1, §9–§12, Status and Evidence to implemented and
  independently verified for the documented deterministic minimal-slice scope, and recorded
  M3 completion subject to the t7 promotion audit. No contract address, bit layout, limit,
  command, typed boundary or deferred item changed.
- **2026-11-16 (t7 promotion audit).** The independent t7 promotion audit returned PASS on
  2026-11-16; the verdict is recorded in the ignored
  `.research/m3-hardware/final-verification/SUMMARY.txt` and the completion is recorded in
  commit `662d06e`. M3 completion is final for the documented deterministic minimal-slice
  scope; no contract address, bit layout, limit, command, typed boundary or deferred item
  changed.
