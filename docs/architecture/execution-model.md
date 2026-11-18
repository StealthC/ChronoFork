# Execution Model


> **Phase 8.2 supersession:** the instruction-quantum statements in this page are
> superseded for the Mega Drive slice. Canonical time is the master-clock cycle:
> each instruction advances the backend by `InstructionOutcome.cycles * 7` and one
> scanline is exactly 3420 master cycles. See
> [Mega Drive slice timing](megadrive-slice.md#deterministic-interrupt-scheduler)
> and [cycle differential evidence](../research/cpu-cycle-differential-evidence.md).
> The regenerated 8-ROM differential matches 8/8 address projections, per-instruction
> cycle costs and selected final states, with 8/8 negative controls.

## Virtual time

Emulated time is a `Tick`: a monotonically increasing `u64` quantum defined by the
backend. It may represent an instruction, a frame, a master-clock cycle or a
device-specific step. ChronoFork does not use host wall-clock time as an implicit
input to hardware behavior.

`Tick::checked_add` and `Tick::next` report overflow instead of wrapping. The
synthetic backend increments one tick per executed instruction.

## Stepping and input

The bootstrap advances one backend-defined quantum per `step`. `InputFrame` is a
compact backend-defined button mask. The synthetic backend consumes it only on its
`APPLY_INPUT` instruction, which makes input timing explicit and testable.

This is intentionally not a full controller model. The v1 model is one backend-defined
mask per step; schema 2 adds ports, analog axes and a keyboard bitmap for the synthetic
backend (below). Remaining future work:

- deeper controller protocol beyond the verified TH-high 3-button reads (A/Start, TH
  latch/6-button, controller timing, analog/keyboard hardware, port 3);
- input scheduled by emulated time or as per-step/scripted events, not host arrival time;
- a versioned input schema shared with TAS records. [ADR-0004](../adr/0004-tas-movie-format.md)
  defines v1 movies as one backend-defined `u32` mask per step; [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md)
  defines the implemented schema 2 for the deterministic synthetic backend.

**Implemented and verified schema 2 (t7 PASS, 2026-11-15).**
[ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) adds a bounded
`InputFrame` with four 32-bit digital ports (port 0 keeps the v1 mask), four signed 16-bit
analog axes and a 64-bit keyboard bitmap, one frame per step. The v1 constructor
`new`, `buttons()` accessor and neutral behavior are preserved; `with_devices`,
`ports()`, `axes()` and `keys()` expose the multi-device fields. The synthetic backend
consumes non-neutral fields only on its existing `APPLY_INPUT` instruction in fixed
ascending order (ports, axes, keyboard bytes), so v1 frames keep their exact state hashes
(a golden pre-schema-2 hash and 0..=255 mask equivalence are verified). The Mega Drive
slice's controller reads are a separate verified model (below); schema 2 remains the
generic multi-device input schema, not a cross-backend controller layout or real
controller-hardware claim. Evidence: M3-A22 in the [format-v2 acceptance
plan](../quality/m3-format-v2-acceptance.md), raw artifacts under
`.research/m3-v2/verification/` and the dated [quality gates](../quality/quality-gates.md)
result.

**Implemented and independently verified: Mega Drive controller input (t5 PASS,
2026-11-16).** [ADR-0009](../adr/0009-real-hardware-input-tas.md) is accepted, implemented
and verified for the deterministic minimal-slice scope. The backend reads the Mega Drive
controller data registers `0xA10003`/`0xA10005` from `InputFrame` ports 0 and 1 with
stateless per-step semantics: the input is visible only during the executed step, is
cleared on every exit path including typed failures, and is never serialized into the
snapshot or state hash. The canonical payload (`CFMD68K2` v2), state body, state hash and
configuration hash are unchanged, so a restored machine uses the next step's input; the
differential probe, demos, M1 oracle/state probes and all existing M2/M3 movie fixtures are
byte-identical to baseline. The verified subset is the TH-high 3-button phase
(Up/Down/Left/Right/B/C, active-low; neutral `0x7F`) on ports 1–2; A, Start, TH/6-button
multiplexing, controller timing, analog and keyboard hardware and port 3 are deferred.
Evidence: [M3 hardware acceptance plan](../quality/m3-hardware-acceptance.md) rows
M3-A45–A53 `executed - passed`, ignored `.research/m3-hardware/verification/` and the dated
[quality gates](../quality/quality-gates.md) result.

## State hashing

Every step returns a `StateHash` (SHA-256) over the backend's canonical mutable
state. The synthetic backend hashes PC, accumulator, RAM, tick, step count and halt
flag. Hashing is used for:

- replay verification;
- fork equivalence at the checkpoint;
- divergence detection between branches;
- snapshot integrity checks.

A hash is only as meaningful as the canonicalization behind it. A real backend
must document exactly which state is included and must include hidden device state
that can affect future execution.

## Determinism

Determinism means: identical ROM/configuration, identical initial state and
identical input sequence at identical ticks produce an identical state hash and
identical observable outputs.

Current evidence (synthetic backend and the minimal Mega Drive controller-read slice):

- two fresh machines given the same input sequence produce equal step outcomes and
  equal final hashes;
- three forks from one checkpoint diverge under different inputs;
- restoring a checkpoint and replaying the same inputs reproduces the branch hash;
- a source session is unchanged by branch execution;
- a recorded v1 movie replays every recorded tick and state hash and reports the first
  divergence exactly (M3-A3 and M3-A4), and identical recordings are byte-identical
  across time, roots and debug/release (M3-A1 and M3-A8);
- a v2 movie round-trips its metadata, embedded checkpoint, ordered anchors and schema-2
  step records deterministically, and independent v2 recordings and anchored replays are
  byte-identical across separate processes and debug/release (M3-A18, M3-A25);
- an in-process rewind through the bounded state cache reproduces the recorded tick and
  state hash at every recorded tick, equal to fresh `Movie::verify`/`ReplayRange` replay,
  and the `tas studio` blocks (including `cache_entries`/`cache_bytes`/`cache_evictions`)
  are byte-identical across separate processes and debug/release (M3-A29, M3-A31);
- an input edit copies and verifies the unchanged prefix, re-simulates the suffix
  deterministically, recomputes every expectation and emits a new branch artifact with
  the source movie/value/file unchanged; truncation does not execute the dropped suffix,
  rebuilt anchors use the canonical fixed identity, and repeated edits are byte-identical
  across processes and in one manager (M3-A35–A41, t10 repair-round-2 PASS 2026-11-16);
- the minimal Mega Drive slice reads the per-step controller frame deterministically: two
  fresh-process recordings with the same ROM/steps/masks are byte-identical, debug and
  release movie bytes agree (`3e11e762…`), 23/23 per-step state hashes are identical for
  neutral vs extreme frames on a non-controller ROM, and pre-change snapshots restore on the
  post-change tree with payload/state/config equality (M3-A45, M3-A46, M3-A50, t5 PASS
  2026-11-16).

These tests do **not** establish determinism for a complete real Mega Drive backend. That
requires differential tests against a trusted reference, including floating-point audio
paths and device scheduling; the controller-read slice is deterministic but is not a
controller-protocol or game-compatibility claim. M3 is complete for the documented
deterministic minimal-slice scope.

## Replay

A replay is a checkpoint plus an ordered input sequence keyed by tick. The host
does not store a parallel "expected state" as truth; it recomputes and compares
state hashes. Divergence is reported at the first mismatching tick.

**Implemented v1 contract (deterministic synthetic backend, independently verified
2026-11-15).** [ADR-0004](../adr/0004-tas-movie-format.md) fixes the canonical
single-file `.cfmovie` format: the exact canonical `Snapshot::to_bytes()` bytes are
embedded as the initial checkpoint, followed by the movie metadata and ordered per-step
records of `(outcome tick, input mask, expected state hash)`. Replay imports the
embedded checkpoint into a fresh session through the existing snapshot path, applies
the recorded inputs, recomputes state, and reports the first divergence as the tick,
input index, expected hash and observed hash. Recording captures the source checkpoint
and executes a separate imported session; the source session is never advanced or
mutated. Host timestamps are absent from canonical movie bytes.

The v1 format, recorder, player, first-divergence report and `chronofork tas
record/play/verify` CLI are implemented and independently verified for the
deterministic synthetic backend (M3-A1–A12, M3-T4 PASS 2026-11-15; see the [M3
acceptance plan](../quality/m3-tas-replay-acceptance.md)). The byte layout, limits,
compatibility matrix and deferred list live in [ADR-0004](../adr/0004-tas-movie-format.md);
this document does not restate them. A recording can also be seeded by an M2 committed
checkpoint, and the resulting movie verifies after the storage root is removed.

**Implemented ranged replay, seek and branch comparison (t5 PASS, 2026-11-15).** The same
v1 movie can be replayed over an explicit `(from, until]` tick window whose boundaries are
the start tick, the end tick or an existing recorded outcome tick; a boundary that is
before the start, after the end, inverted, unaligned or empty is a typed rejection before
any checkpoint import, session creation or step. Every ranged replay re-simulates linearly
from the embedded checkpoint in a fresh session — O(n) in the `until` boundary index,
bounded by the v1 step limit — and verifies the whole prefix as well as the window;
`chronofork tas play|verify --from/--until` prints the summary block plus `range_from`,
`range_until`, `prefix_steps`, `window_steps` and `replayed_steps` (no per-step blocks).
In the ranged-replay surface, forward single-tick advance and rewind are new invocations,
not carried state: each re-simulates from the embedded checkpoint and reports the same
outcome as a first-ever invocation at the same range. Branch comparison (`chronofork tas compare
--movie/--against`) reads the recorded `(outcome tick, state hash)` records by index and
never re-simulates; it reports the first divergent index, tick and hash, and reports
step-count and embedded-checkpoint (start tick/state hash) mismatches separately, with
metadata and input masks never deciding equality. A compare PASS is not a verification;
`verify` remains the re-simulation path. Evidence: M3-A13–A17 in the [increment-2
acceptance plan](../quality/m3-partial-replay-acceptance.md) (t5 PASS, 2026-11-15), raw
artifacts under the ignored `.research/m3-tas-inc2/verification/` path, and the dated
[quality gates](../quality/quality-gates.md) result. [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md)
is the canonical contract.

Not implemented: checkpoint-reference movies, the rest of the real-hardware controller
protocol beyond the verified TH-high 3-button reads (A/Start, TH latch/6-button, controller
timing, analog/keyboard hardware, port 3), import/export adapters,
compression/encryption and HTTP/MCP/GUI/daemon. The in-process rewind state cache and
interactive stepping contract is implemented and verified below
([ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md)); rerecord and
branch editing is implemented and verified below
([ADR-0008](../adr/0008-rerecord-and-branch-editing.md)); real-hardware input TAS is
implemented and verified under [ADR-0009](../adr/0009-real-hardware-input-tas.md).

**Implemented and verified v2 anchors and anchored seek (t7 PASS, 2026-11-15).**
[ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md) adds the
`CFMOVIE2` container: the initial canonical checkpoint, an ordered table of canonical
checkpoint anchors taken after recorded steps, 72-byte schema-2 step records and a
trailing SHA-256. Each anchor is parsed only through `Snapshot::from_bytes` and bound at
decode/construction to its recorded step's outcome tick and expected state hash. A ranged
replay over a v2 movie selects the nearest anchor at or before the requested prefix end,
imports that anchor through `SessionManager::create_session_from_snapshot_bytes`, and
executes only the later steps up to `until`. It reports `anchored=1`, `anchor_step`,
`anchor_tick` and `prefix_verified=0` when an anchor is used; movies without anchors (all
v1 and anchor-less v2 movies) keep the linear path. The deliberate delta is that an
anchored ranged replay does not re-execute or re-verify the pre-anchor prefix; full-range
`verify` still executes every step and remains the complete verification path. The v2
limits are 2,000,000 steps, 65,536 anchors and 48 MiB cumulative anchor snapshots inside
the existing 256 MiB file bound. Evidence: M3-A18–A21 and M3-A24–A25 in the [format-v2
acceptance plan](../quality/m3-format-v2-acceptance.md), raw artifacts under
`.research/m3-v2/verification/` and the dated [quality gates](../quality/quality-gates.md)
result. Real-hardware input TAS is implemented and verified for the TH-high 3-button
subset (M3-A45–A53); M3 is complete for the documented deterministic minimal-slice scope,
with the remaining deferred items listed under Known limitations.

**Implemented and verified in-process rewind state cache and interactive stepping (t5 PASS,
2026-11-15).** [ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md) adds a
bounded, session-layer cache of canonical states captured after verified recorded steps,
keyed by recorded step index and held only for the process lifetime. Rewinding to an exact
recorded tick restores the greatest verified cached state at or before the target — or the
embedded checkpoint when the target precedes the earliest cached state, including after
eviction or with caching disabled — and replays the recorded inputs forward through the
target, verifying every recorded expectation and reporting the exact first divergence. The
cost is proportional to the distance from the chosen start state: there is no O(1),
random-access or reverse-execution seek, no interpolation, and no state carried across
invocations. The interactive surface is a stdin-driven `chronofork tas studio --movie
<FILE>` process that owns exactly one explicit in-process player, verifies expectations on
every forward step, prints stable `key=value` blocks and writes no storage root, cache or
file. The cache uses a crate-private session capture/restore path that allocates no snapshot
identity and does not grow the session checkpoint map; v2 anchors are not used as cache
seeds. Defaults are a one-step capture interval, 256 entries and 16 MiB, with maxima of
4,194,304 interval, 65,536 entries and 64 MiB. Evidence: M3-A28–A31 and M3-A33 in the
[M3 rewind acceptance plan](../quality/m3-rewind-acceptance.md) (t5 PASS, 2026-11-15), raw
artifacts under the ignored `.research/m3-rewind/verification/` path and the dated
[quality gates](../quality/quality-gates.md) result. The acceptance claim is the
deterministic synthetic backend on Linux; no persistent or cross-process cache, hardware
rewind or O(1) seek is claimed.

**Implemented and independently verified rerecord and branch editing (t10 repair-round-2
PASS, 2026-11-16).** [ADR-0008](../adr/0008-rerecord-and-branch-editing.md) adds
`chronofork tas edit` and the session-layer `chronofork_sessions::edit` implementation.
An edit imports the source movie's embedded checkpoint into a fresh session, executes and
verifies the unchanged prefix against its recorded expectations, applies replacement
input frames to one or more existing recorded steps, optionally truncates after a chosen
step, re-simulates the suffix deterministically, recomputes every expectation and
constructs a new canonical movie of the **same** format and input-schema version (a v1
source produces a v1 output, a v2 source a v2 output; no migration, upgrade or
downgrade). The source movie value, source file and any pre-existing session are
unchanged. The output is a new branch artifact with its own non-zero movie id and
`parent_movie_id` set to the source id; the parent is passive metadata under the existing
dangling-parent rule and is never resolved. The embedded checkpoint is copied unchanged.
Anchors before the earliest affected step are retained verbatim; anchors at or after it
are dropped or rebuilt at the v2 cadence, with rebuilt envelopes re-encoded under the
canonical fixed snapshot/origin identity so an unrelated session or prior edit cannot
change a later edit's bytes. No second anchor format and no anchor-based prefix skipping
exist. No new canonical field, format version or persisted rerecord counter was added:
the command prints per-invocation `edits`/`truncate_after`/`prefix_steps`/
`resimulated_steps` diagnostics, and the parent edge is the provenance. Existing `play`,
`verify`, ranged replay, `compare` and `studio` accept the output unchanged. Evidence:
M3-A35–A44 in the [M3 rerecord acceptance
plan](../quality/m3-rerecord-acceptance.md), raw artifacts under the ignored
`.research/m3-rerecord/verification-round-2/` path, and the dated
[quality gates](../quality/quality-gates.md) result. The synthetic-backend claim is
Linux-hosted and deterministic; real-hardware input TAS is implemented and verified for
the TH-high 3-button subset, while in-place/timeline editing, persisted counters,
import/export and O(1) seek remain unimplemented. M3 is complete for the documented
deterministic minimal-slice scope.

## Snapshot format

The outer envelope is defined in `chronofork-sessions::snapshot`:

```text
magic "CFSNAP01"
u16 outer format version
u64 snapshot id
u64 origin session id
u16 backend id length + bytes
u16 backend version length + bytes
u16 backend payload format version
u64 tick
32-byte config hash
32-byte state hash
u64 payload length + payload
32-byte SHA-256 checksum over all preceding bytes
```

The payload is opaque. Decoding validates magic, version, lengths, UTF-8 strings,
checksum and trailing bytes. Version mismatch is an explicit error, not a
best-effort parse.

## Host time and nondeterminism policy

- Host time may be used for logging, timeouts and UI, never as an implicit hardware
  input.
- Randomness must be an explicit, seedable service if a backend needs it.
- Floating-point behavior must be tested on every supported platform before
  claiming cross-platform determinism.
- Scheduling across threads must not change the emulated tick ordering. The
  bootstrap avoids that problem by being caller-driven.
- When determinism cannot be guaranteed, the limitation must be documented and the
  capability must not claim otherwise.

## Known limitations

- No fully cycle-accurate hardware timing, DMA, timer/Z80 interrupts or audio. The
  Mega Drive slice uses a 3420-master-cycle scanline timeline, with 68000 interrupt
  entry on that timeline, SR-mask gating, `RTE` handler return, VDP VINT-pending
  wiring, privilege enforcement for the implemented system instructions and
  exception vectors 1-9/24-31/32+n. Remaining limits: functional prefetch, bus
  contention and DMA stalls are not cycle-modelled; only HBlank (level 4) and
  VBlank (level 6) sources are modelled, with level 7 a predicate rule rather than
  a wired source; no interrupt-latency equivalence is claimed; and VDP
  interrupt-enable/acknowledge logic is absent.
- No general structured/timestamped trace format, conditional breakpoints or
  watchpoints. A narrow debug-only instruction `index`/`pc`/`opcode` trace and an
  external oracle address-stream comparison exist, but no bus/device trace,
  historical query or breakpoint surface.
- TAS movie v1 and v2 are implemented and verified for the deterministic synthetic backend;
  the TH-high 3-button controller reads and `--rom` hardware TAS are implemented and
  independently verified on the minimal Mega Drive slice (M3-A45–A53, t5 PASS 2026-11-16).
  Ranged replay/seek, forward single-tick advance and rewind by deterministic linear
  re-simulation from the embedded checkpoint (O(n), typed range bounds) and branch
  comparison of recorded `(tick, state hash)` records (no re-simulation) are implemented
  and verified (M3-A13–A17, t5 PASS 2026-11-15). Movie format v2 checkpoint anchors,
  anchor-based seek and schema-2 multi-device input are implemented and independently
  verified for the deterministic synthetic backend (M3-A18–A27, t7 PASS 2026-11-15;
  [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md),
  [format-v2 acceptance plan](../quality/m3-format-v2-acceptance.md)). Rerecord and branch
  editing is implemented and independently verified for the deterministic synthetic
  backend (M3-A35–A44, t10 repair-round-2 PASS 2026-11-16;
  [ADR-0008](../adr/0008-rerecord-and-branch-editing.md),
  [M3 rerecord acceptance plan](../quality/m3-rerecord-acceptance.md)). Real-hardware input
  TAS is implemented and independently verified for the TH-high 3-button subset
  ([ADR-0009](../adr/0009-real-hardware-input-tas.md); M3-A45–A53, t5 PASS 2026-11-16), with
  the canonical state/demos/M1 evidence unchanged. Deferred and unimplemented:
  checkpoint-reference movies, the deeper controller protocol (A/Start, TH/6-button,
  timing, analog/keyboard, port 3), import/export adapters, in-place/timeline editing,
  persisted rerecord counters, mixed anchors, compression/encryption and
  HTTP/MCP/GUI/daemon.
- Branch comparison is a recorded-expectation diff, not a verification or determinism
  proof; it cannot detect a build whose emulation changed, and metadata is never an
  equality field.
- No frame pacing or host-time run loop. The in-process rewind state cache and
  `chronofork tas studio` stepping surface are implemented and verified
  ([ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md); M3-A28–A34), but
  rewind is restore-plus-replay from a verified cached state or the embedded checkpoint
  (bounded re-simulation; no O(1)/random-access or reverse-execution seek), the cache is
  process-local and per-player (no persistent, shared or cross-process cache), and the
  claim is the deterministic synthetic backend on Linux.
- No cross-platform determinism claim beyond repeated runs on the current host.
- The synthetic backend state hash covers only synthetic state; the real Mega Drive
  slice hash covers CPU, RAM, VDP and scheduler state (`CFMDSTATE2`). A cross-backend
  canonicalization remains undefined.
