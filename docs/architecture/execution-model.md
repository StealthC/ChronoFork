# Execution Model

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

This is intentionally not a full controller model. Future versions need:

- multiple ports and controller types;
- analog axes and device-specific peripherals;
- input scheduled by emulated time, not host arrival time;
- a versioned input schema shared with TAS records.

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

Current evidence (synthetic backend only):

- two fresh machines given the same input sequence produce equal step outcomes and
  equal final hashes;
- three forks from one checkpoint diverge under different inputs;
- restoring a checkpoint and replaying the same inputs reproduces the branch hash;
- a source session is unchanged by branch execution.

These tests do **not** establish determinism for a real Mega Drive backend. That
requires differential tests against a trusted reference, including floating-point
audio paths and device scheduling.

## Replay

A replay is a checkpoint plus an ordered input sequence keyed by tick. The host
does not store a parallel "expected state" as truth; it recomputes and compares
state hashes. Divergence is reported at the first mismatching tick.

No TAS movie format exists yet. The bootstrap proves the primitives a movie format
will use, not the format itself.

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

- No cycle-accurate hardware timing, DMA, timer/Z80 interrupts or audio. The Mega
  Drive slice has an instruction-quantum VBlank/HBlank scheduler, and 68000
  interrupt entry with SR-mask gating, `RTE` handler return and VDP VINT-pending
  wiring implemented for the modelled slice. Remaining limits: privilege is not
  enforced; only HBlank (level 4) and VBlank (level 6) sources are modelled, with
  level 7 a predicate rule rather than a wired source; no interrupt-timing
  equivalence is claimed; and VDP interrupt-enable/acknowledge logic is absent.
- No general structured/timestamped trace format, conditional breakpoints or
  watchpoints. A narrow debug-only instruction `index`/`pc`/`opcode` trace and an
  external oracle address-stream comparison exist, but no bus/device trace,
  historical query or breakpoint surface.
- No frame pacing, rewind or seek.
- No cross-platform determinism claim beyond repeated runs on the current host.
- The synthetic backend state hash covers only synthetic state; the real Mega Drive
  slice hash covers CPU, RAM, VDP and scheduler state (`CFMDSTATE2`). A cross-backend
  canonicalization remains undefined.
