# ADR-0017: VDP read cache (`0x0C` 8-bit VRAM read) and payload v8

- **Status:** **Accepted** — owner direction recorded 2026-11-24: "autorizo você a seguir com
  o plano recomendado", which authorises Option B (the faithful read cache with `CFVDP003` and
  payload v8) and the ADR-first sequence. Implemented in the same change (§"Implementation
  status"). Acceptance is the design decision; it does not certify hardware agreement beyond the
  pinned oracle (see "What is NOT proved").
- **Date:** 2026-11-24.
- **Milestone:** M8 (functional Mega Drive emulator) — VDP fidelity follow-up after W1.8.
- **Related:** [ADR-0002](0002-backend-boundaries.md) (backend boundaries),
  [ADR-0016](0016-session-service-and-agent-api.md) (the byte/count quota preflight that consumes
  `snapshot_payload_bound`), [ADR-0015](0015-verification-contract.md) (independent verification).
  Supersedes no ADR; it extends the payload-versioning rule stated in
  [backend-model](../architecture/backend-model.md) and implemented in
  `crates/chronofork-megadrive/src/backend.rs`.
- **Input evidence (t4):** [`.research/0c/oracle-notes.md`](../../.research/0c/oracle-notes.md)
  (read-only decode of the pinned Exodus core `ec0056151f3ff1c334d09ee375abb30a68fdfec5`,
  with line-numbered source copies in `.research/0c/source/`, executed probe/MCP runs in
  `.research/0c/commands.md`, and the 13 test vectors in its §4). The design elaboration is in
  [`.research/0c-v8/design.md`](../../.research/0c-v8/design.md).

## Context

The Mega Drive VDP data port decodes the read target from the **low five bits** of the command
code. This slice implements `0x00` (16-bit VRAM), `0x04` (VSRAM) and `0x08` (CRAM) reads; `0x0C`
is the documented but **undocumented-by-Sega** 8-bit VRAM read, and today it is a deliberate
typed rejection: `VdpError::UnsupportedBitRead { code: 0x0C }`
(`crates/chronofork-megadrive/src/vdp/mod.rs:1432-1434`; the constant is documented at
`crates/chronofork-megadrive/src/vdp/regs.rs:95-100`, and the behaviour is pinned by
`crates/chronofork-megadrive/tests/vdp_ports.rs:213-249`).

Task t4 decoded the pinned oracle's exact behaviour and established that a faithful
implementation needs hardware state this slice does not model at all: the VDP's 16-bit **read
buffer** and its two flags. Concretely (t4 `oracle-notes.md` §1, PROVADO, with source lines):

- `0x0C` reads **one** VRAM byte at the command address with **bit 0 inverted**
  (`Devices/315-5313/S315-5313_General.cpp:3011-3027`) and writes it into the **low byte** of
  the read buffer (`SetByteFromBottomUp(0, …)`); the high byte is left untouched
  (`:2448-2449`, `:3023-3026`).
- The read buffer is seeded from the write FIFO's next-read entry whenever
  `_readDataHalfCached` is false (`:2395-2398`); `0x0C` never sets that flag.
- A successful cache operation sets `_readDataAvailable` (`:2451`); the data-port read returns
  the whole buffer (no mask) and clears **both** flags (`S315-5313_Ports.cpp:1078-1084`); a
  control-port write invalidates the cache and reloads the internal address copy
  (`:1605-1617`).
- The command address increments once per completed read (`:2473-2477`), and the cache
  operation runs only while the write FIFO is empty (`:1748`, `:1850-1856`).
- The 68000 assembles word reads from 16 bus lines, selects the byte lane by address parity, and
  issues long reads as two word reads (`Devices/M68000/M68000.cpp:1986-2055`); the Z80 portal
  `0x7F00..=0x7F03` maps to `0xC00000 | offset` and returns the upper half for an even and the
  lower half for an odd portal byte
  (`Devices/MD1600IO/MDBusArbiter.cpp:347-384`; modelled at
  `crates/chronofork-megadrive/src/bus.rs:1950-2030`).

The high half of a `0x0C` result is therefore **not derivable** from VRAM, CRAM, VSRAM, the
registers and the command address: it is the previous 16-bit read value (or the FIFO seed). A
checkpoint taken between the two reads of a 68000 word/long access must carry it to resume
identically, so this is canonical state, not a derived view.

Two further facts constrain the change:

1. The VDP snapshot is a **versioned sub-format inside the payload**: `CFVDP002`, version 2,
   `SNAPSHOT_LEN = 65_811` (`vdp/mod.rs:157-159`, pinned by
   `crates/chronofork-megadrive/tests/vdp_snapshot.rs:106-108`). The current 10-byte layout
   header uses only six of the eight flag bits; bits 6 and 7 are reserved and their presence is
   rejected (`SNAPSHOT_FLAG_MASK` at `vdp/mod.rs:204-218`, `restore_bytes` at `:1301-1305`,
   test at `vdp_snapshot.rs:245-249`).
2. The payload embeds that snapshot verbatim (`backend.rs:1782`, `VDP_SNAPSHOT_LEN` at
   `backend.rs:151`), and the W1.8 declared bound is a compile-time function of the payload
   constants: `SNAPSHOT_PAYLOAD_BOUND = PAYLOAD_MAGIC.len() + 2 + FIXED_STATE_BODY_LEN +
   SCHEDULER_MAX_SNAPSHOT_LEN = 206_275` (`backend.rs:213-241`). Any change to the VDP snapshot
   length moves `FIXED_STATE_BODY_LEN`, the payload length, the declared bound, the golden
   hashes and the payload-version rejection set.

## Problem

How should the `0x0C` 8-bit VRAM read be implemented faithfully, and how should the canonical
read-buffer state it introduces be versioned, validated, bounded and tested, without
reinterpreting any existing serialized state?

## Decision

### 1. Model the read cache as canonical VDP state (three new fields)

Add to `Vdp` (`vdp/mod.rs:514-…`):

| Field | Type | Meaning (pinned source) |
|---|---|---|
| `read_buffer` | `u16` | the 16-bit value the data port will return (`_readBuffer`) |
| `read_data_available` | `bool` | a fresh value is latched for the current command (`_readDataAvailable`) |
| `read_data_half_cached` | `bool` | only the first half of a 16-bit VRAM read has been latched (`_readDataHalfCached`) |

All three default to `0`/`false` on a fresh device, matching the pinned reset block
(`S315-5313_General.cpp:333-351`: FIFO entries `dataPortWriteData = 0`,
`_readDataAvailable = false`, `_readDataHalfCached = false`).

The data-port read becomes the faithful two-step model of
`PerformReadCacheOperation` + the port read; the exact state machine (ordering, FIFO gate,
`0x0C` versus `0x00`) is specified step by step in
[`design.md` §2](../../.research/0c-v8/design.md) so the implementation does not have to
re-derive it from the oracle.

### 2. `0x0C` semantics (normative)

The cache gate is **global**: it precedes the target decode, exactly the oracle predicate
`IsWriteFIFOEmpty() && !_readDataAvailable && ValidReadTargetInCommandCode()`
(`General.cpp:1748`, `:1850-1856`), and applies to `0x00`, `0x04`, `0x08` and `0x0C`. It gates
**only the cache refresh**: the port-return steps (clear `read_data_available` and
`read_data_half_cached`, advance the command address once) run on every accepted data-port read,
including when the refresh is skipped because a write is pending or a value is already cached.
The `0x0C`-specific rules follow:

1. The cache step refreshes nothing — and drains nothing — when the write FIFO is **not**
   empty or a value is already latched. This is a **cache-step-local** return: the port read
   still returns the buffer's current value, and the port-return steps (rule 3) still clear both
   flags and advance the address exactly once. Nothing is fabricated (the upstream lock-up
   region at `:2352-2377` stays a documented open question, `oracle-notes.md` §5).
2. Otherwise, if `!read_data_half_cached`, seed `read_buffer` from the FIFO's next-read entry
   (`:2395-2398`); then set `read_buffer`'s **low byte** to `vram[address ^ 1]` — bit 0 of the
   literal command address inverted, exactly as `M5ReadVRAM8Bit` does (`:3011-3027`) — leave the
   high byte untouched, set `read_data_available = true`, and leave `read_data_half_cached`
   false.
3. After the port read returns, clear `read_data_available` and `read_data_half_cached`
   (`Ports.cpp:1083-1084`) and add the auto-increment register to the command address exactly
   once (`:2474-2477`).

The `0x00` case keeps its current external behaviour and is re-expressed on the same read buffer
so the following `0x0C` seed is gated as the pin gates it: this slice performs **one atomic word
access** (the pin's two-port half-read is not exposed), leaves the word in the buffer and sets
`read_data_half_cached = true`, which the data-port read then clears — so the next completed access
seeds normally. This is a disclosed model abstraction (`design.md` §2.1), not a pin-exact claim.
The 68000 **word**, **byte** and **long** paths and the Z80 portal path are described in §3; the
address increment happens once per *VDP data-port operation*, never once per byte lane.

### 3. Access paths (normative consequences)

- **68000 word** (`0xC00000`, even offset): one VDP data-port read, full 16-bit buffer returned,
  one increment (`M68000.cpp:2010-2025`).
- **68000 byte** (`0xC00000` or `0xC00001`): one VDP data-port read; the returned lane is the
  buffer's high byte for an even address and its low byte for an odd one; one increment
  (`:1986-2007`; already the shape of `Vdp::read8`, `vdp/mod.rs:1124-1137`).
- **68000 long**: two word reads at `location` and `location + 2` → **two** VDP data-port
  reads and **two** increments (`:2026-2055`; existing bus shape at `bus.rs:1069-1071`).
- **Z80 portal `0x7F00..=0x7F03`**: each portal byte is one VDP data-port operation
  (`0x7F00`/`0x7F02` → data port, `MDBusArbiter.cpp:352`); the even portal bytes
  (`0x7F00`, `0x7F02`) return the buffer's high byte and the odd ones (`0x7F01`, `0x7F03`) its
  low byte, with one increment per access (`:377-379`; the slice's four-byte window is
  `bus.rs:204-205` and its read path `:1950-1985`). No new Z80 addressing is introduced: only
  the byte-lane outcome inside the already-mapped window changes.
- The capability-gap classification for `0x0C`
  (`VdpError::UnsupportedBitRead` → `GuestAccessClass::MissingCapability`, `bus.rs:241`, and the
  Z80 `count_capability_gap()` path, `bus.rs:1973-1976`) is **removed** for `0x0C`: after this
  change `0x0C` is an implemented operation with no error outcome. The `UnsupportedBitRead`
  variant and its counter must not be silently repurposed: implementation either deletes the
  variant with its tests updated (`vdp_ports.rs:213-249`, `bus.rs:241`, `:1973-1976`) or keeps it
  documented as unused-by-`0x0C`. Removing a dead public-ish error arm is preferred and is part
  of the implementation task, not of this ADR.

### 4. VDP snapshot `CFVDP003`, `SNAPSHOT_VERSION = 3`

New layout (all multi-byte integers little-endian), extended at the tail so existing offsets are
untouched:

| Offset | Field |
|---|---|
| `0..8` | magic `CFVDP003` |
| `8..10` | version `3` |
| `10` | flags: bit 0 VBlank, bit 1 half-latched command, bit 2 odd frame, bit 3 sprite collision, bit 4 sprite overflow, bit 5 PAL, **bit 6 `read_data_available`**, **bit 7 `read_data_half_cached`** |
| `11` | command code (six bits) |
| `12..14` | command address |
| `14..38` | registers (24) |
| `38..166` | CRAM (64 words) |
| `166..246` | VSRAM (40 words) |
| `246` | FIFO occupancy |
| `247..255` | FIFO words oldest first (4 words, zero-padded) |
| `255` | DMA active |
| `256..260` | DMA source |
| `260..262` | DMA length |
| `262` | DMA mode code |
| `263..271` | frame index |
| `271..273` | line |
| `273..275` | dot |
| `275..275+VRAM_SIZE` | VRAM (64 KiB) |
| **new tail** `65_811..65_813` | **`read_buffer` (`u16` LE)** |

So `SNAPSHOT_LEN = 65_813` (+2). Bits 6 and 7 are **no longer reserved**: they are assigned to
the two read flags and stay inside the validated `SNAPSHOT_FLAG_MASK`, so the existing reserved
bit rejection continues to reject bits that remain unassigned (none today) and the validator
must now accept `0x40`/`0x80` as defined values rather than rejecting them.

Validation and rejection order (same contract as today, one version later):

1. `SnapshotLengthMismatch { expected: SNAPSHOT_LEN, found }` — exact length first.
2. `SnapshotMagicMismatch` — `CFVDP002` and `CFVDP001` both rejected on the magic (a v2 snapshot
   is never reinterpreted as v3).
3. `SnapshotVersionUnsupported { found }` — a `CFVDP003` magic with a version field other than 3
   is rejected by version, so the error names the mismatch.
4. `SnapshotFlagsInvalid { flags }` — any flag bit outside the new mask.
5. The existing field validations (command code width, FIFO occupancy, DMA mode, line/dot).
6. `read_buffer` is a plain `u16` read after the VRAM copy; there is no invalid value for it.

The rejection order is deliberately the current one so the existing negative tests keep their
meaning; the only behavioural change is that `CFVDP002` moves from "accepted" to "rejected by
magic" (asserted explicitly), and `0x40`/`0x80` move from "rejected" to "defined".

**No silent reinterpretation**: there is no upgrade path that reads v2 bytes as v3. Migrating a
session recorded under v7/payload v7 requires the explicit path that already exists for
incompatible input — reject and re-create (or re-run) the state; the implementation must not add
a lenient parser, and must not accept a short/long payload by padding.

### 5. Payload/state/config v8

| Constant | v7 | v8 |
|---|---|---|
| `PAYLOAD_VERSION` | 7 | **8** |
| `PAYLOAD_MAGIC` | `CFMD68K7` | **`CFMD68K8`** |
| `STATE_MAGIC` | `CFMDSTATE7` | **`CFMDSTATE8`** |
| `CONFIG_MAGIC` | `CFMDCONF7` | **`CFMDCONF8`** |

`parse_payload` keeps its order: **magic first, then version** (`backend.rs:1745-1757`), so a
v1–v7 payload is rejected by its own magic and a v8 magic with an old version field is rejected
by the version check. Both errors stay `BackendError::InvalidSnapshot(String)` with the existing
message shapes ("Mega Drive payload magic mismatch", "Mega Drive payload version {version},
expected {PAYLOAD_VERSION}"). The version-rejection test grows a v7 row (and keeps v1–v6), and
the same magic/version split must be exercised for v7 exactly as for the earlier versions
(`backend.rs:3396-3447`).

The **state hash** changes because it is `hash(STATE_MAGIC + state body)` and the body grew by
the VDP snapshot growth (`backend.rs:1703-1708`, assertion at `:4081-4087`). The **configuration
hash** changes only because `CONFIG_MAGIC` changed; the configuration inputs (ROM, video
standard, TMSS, features, SRAM) do not change, so no `CONFIG_FEATURE_TAG` bump is justified.

### 6. Re-freeze of goldens, with the cause recorded

`backend.rs:4092-4130` pins payload length, payload hash, state hash and configuration hash for
six demo steps. The re-freeze is a deliberate, reviewed act, and each moved value must record its
cause in the test comment, exactly as the v6→v7 move does today:

| Golden | v7 value | Why it moves in v8 |
|---|---|---|
| payload length | `206_259` | `VDP_SNAPSHOT_LEN` grows by 2 (`read_buffer`) |
| payload hash | `e89067cc…` | the payload bytes changed (snapshot magic/version/flags/tail) |
| state hash | `61c93ed3…` | `STATE_MAGIC` changed and the hashed body grew by 2 |
| configuration hash | `c7731b6c…` | `CONFIG_MAGIC` changed; the configuration itself did not |

Also re-pinned, because they assert format identity rather than a hash:
`crates/chronofork-megadrive/tests/snapshot_v4.rs:19-21` and `snapshot_v5.rs:35-37` (magic
triplets), `audio_snapshot.rs:117-119`, `cartridge_snapshot.rs:64-66`,
`vdp_snapshot.rs:106-108` (`CFVDP003`, `SNAPSHOT_VERSION = 3`, `SNAPSHOT_LEN = 65_813`),
`snapshot_properties.rs:105` (property input length), and every `CFMD68K7`/`CFVDP002` literal in
the tree (grep-verified by the implementation task; `crates/chronofork-cli/tests/w18_payload_bound.rs`
derives its numbers from the format and needs no literal change).

**No golden may be deleted or weakened.** A changed golden is re-pinned with a comment naming the
cause; an unchanged golden that starts failing is a bug, not a re-freeze.

### 7. Re-derivation of the declared bound

`SNAPSHOT_PAYLOAD_BOUND` is derived, never written as a literal (`backend.rs:233-241`). With
`VDP_SNAPSHOT_LEN` +2:

- `FIXED_STATE_BODY_LEN` grows by 2 (it contains `VDP_SNAPSHOT_LEN`, `backend.rs:213-231`);
- fresh payload `206_259 → 206_261`; one pending `206_267 → 206_269`; two pending
  `206_275 → 206_277`; `SNAPSHOT_PAYLOAD_BOUND 206_275 → 206_277`.

The tightness coverage must be re-derived, not widened: the 0/1/2-pending difference stays
exactly 0/8/16 bytes (the scheduler is the only variable component), the two-pending state still
reaches the bound exactly, NTSC and PAL and imported-SRAM sweeps must never exceed it and must
still reach it, a restored widest state must still keep it, and the CLI service+Mega Drive
regression (`crates/chronofork-cli/tests/w18_payload_bound.rs`) must pass under a finite
`max_bytes_per_session` with the counting wrapper proving that refusals precede
`Backend::snapshot()`. If the tightness no longer reaches the bound exactly, that is a defect to
report, not to paper over by enlarging the bound.

### 8. Test plan

The reconciled vectors of t4 `oracle-notes.md` §4 become VDP- and machine-level tests (mapping in
[`design.md` §6](../../.research/0c-v8/design.md)); the original vectors **3 and 8 are superseded**
by the contract in "Reconciled contract" below and are not claimed as stated, plus the negative
controls listed there:
unknown/older VDP versions and payload versions rejected by magic and by version, reserved flag
bits still rejected, unsupported offsets, the `0x0C` write direction still discarded and counted,
and the removed capability-gap classification proving it is gone rather than repurposed. Golden
re-freeze and bound re-derivation are asserted as ordinary tests, and the whole change must pass
`cargo fmt`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `./scripts/check.sh` and `git diff --check`.

## Reconciled contract (CONFIRMED BY OWNER — 2026-11-24)

Two requirements stated when this ADR was accepted were **derived** from t4 rather than observed,
and the pinned source does not support them. The requirement change below is **confirmed**:
CONFIRMED BY OWNER — 2026-11-24, relayed by captain: "Sim — superseder 3/8 e manter 0x00CD
(Recomendado)". The decision confirms (a) the zeroed empty-window seed, (b) the atomic `0x00`
word abstraction and (c) the supersession of the derived t4 vectors 3/8 (`0xEFCD` -> `0x00CD`),
with the approximation documented here and in `design.md` §2.1. Nothing here is claimed as
pin-exact beyond the cited lines.

- **Global gate.** The cache step runs only while the write FIFO is empty — the pin's
  `IsWriteFIFOEmpty() && !_readDataAvailable && ValidReadTargetInCommandCode()` predicate
  (`General.cpp:1748`, `:1850-1856`). With a pending write no read refreshes, drains or
  fabricates; the data port returns the buffer's current value. This is not a `0x0C` special
  case.
- **Seed.** A completed access seeds the buffer from the FIFO's next-read entry when
  `!half_cached` (`:2395-2398`). In this slice the entry of an empty window is the zeroed slot,
  because the FIFO model keeps no processed write, so a `0x0C` read with an empty window returns
  `0x00 || vram[address ^ 1]`. **Disclosed limitation**: the pinned core's entry retention is not
  modelled.
- **Superseded vectors.** t4 vectors 3 and 8 asked for a nonzero previous upper byte to persist
  across a completed `0x00` read and a following `0x0C` (`0xEFCD`). Under the seed above the
  reconciled result is **`0x00CD`**; the design's mapping and its masked `(address & 0xFFFE) + 1`
  formula are corrected to literal `address ^ 1` in
  [`design.md` §2.1](../../.research/0c-v8/design.md). The original vectors are not claimed to
  pass.
- **Atomic `0x00`.** This slice performs one atomic word access for the 16-bit VRAM read instead
  of the pin's two data-port halves, and sets `half_cached` so the following `0x0C` seed is gated
  as the pin gates it. **Disclosed limitation**, not a pin-exact claim.
- **Control writes.** Every successful control write clears both read flags
  (`Ports.cpp:1613-1617`) on every branch; a rejected write mutates nothing.

## Consequences

**Positive:** `0x0C` follows the pinned source: the global cache gate, the `!half_cached` seed and
the literal `address ^ 1` low-byte rule; the read cache becomes a single modelled concept that
also makes `0x00`'s half-cache explicit and removes one capability gap from the honest capability
record; snapshots carry the buffer and flags, and a control write invalidates them. The upper byte
is **not** a retained previous read value: with an empty window the seed is zero (disclosed
limitation, "Reconciled contract").

**Negative:** a format and payload version bump with a golden re-freeze and a bound change; every
v1–v7 payload and `CFVDP002` snapshot becomes invalid input and must be re-created. All accepted
artifacts that state payload v7 (architecture docs) need a one-line follow-up outside this
ADR's scope.

**Neutral/known:** `read_data_available` and `read_data_half_cached` remain unobservable through
any host surface except a checkpoint, exactly like today's internals; the diagnostic counters
(`discarded_data_port_accesses`, `capability_gap_accesses`) keep their meaning except that `0x0C`
no longer feeds the capability-gap counter.

## Alternatives considered and discarded

- **A. Implement `0x0C` without any new canonical state (no version bump).** Returns
  `(0x00 | vram_byte)` and increments once. Discarded: it cannot carry the seed/FIFO-dependent
  buffer or the two flags across a checkpoint (t4 vector 11 and the control-write invalidation
  transition), it makes the 68000 byte-lane, Z80 portal and restore results diverge from the
  modelled device by construction, and it would silently *invent* a value for the high half
  instead of modelling it. It also leaves the W1.8 bound untouched, which looks
  cheaper but buys an incorrect emulator.
- **B. Keep the read buffer out of the snapshot and re-seed deterministically on restore.**
  Discarded: the value is a function of the *previous read*, which is not recoverable from VRAM
  alone; a restore would silently change a subsequent read (exactly the "never silently
  reinterpret bytes" rule inverted into "never silently change state").
- **C. Store the buffer inside the existing snapshot without a version bump (reuse reserved
  bits).** Discarded twice over: the length would change under an unchanged `CFVDP002` identity,
  so a v2 reader would accept-and-misparse; and the versioned-format rule requires the identity
  to move when the layout moves.
- **D. Encode the read buffer in the payload but not in the VDP snapshot.** Discarded: the
  payload embeds the VDP snapshot verbatim; a second serialization of the same VDP state is a
  duplicated format (anti-slop policy) and would drift.
- **E. Also model the hardware lock-up of `0x0C`-after-write.** Discarded for this ADR: the
  pinned source marks it `##TODO##`/unconfirmed (`:2352-2377`), and t4 records it as ABERTO. The
  implementation must stay deterministic and must not fabricate data; the slice keeps the
  documented order and counts discards where it already does.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| A golden is "fixed" by weakening the assertion | Re-freeze protocol of §6: change the pinned value with a comment naming the cause; review checks no assertion is deleted or relaxed |
| The bound is enlarged instead of re-derived | §7: the bound stays a compile-time expression; tightness must still reach it exactly; a failure is reported, not accommodated |
| v2 snapshots accidentally accepted by a lenient parser | Explicit rejection tests for `CFVDP002`/version 2 and for v1–v7 payloads, both by magic and by version field |
| `0x0C` silently inherits the old capability-gap counter | §3: remove the classification for `0x0C`; a test asserts the counter does not move on a successful `0x0C` read |
| The two read flags are mistaken for host-observable status | §4/Consequences: they are canonical state only; the status register mapping is unchanged |
| Read-buffer semantics drift between `0x00` and `0x0C` | One shared buffer + one shared cache step; vectors 3, 8, 11 specifically cross the two codes |

## What is NOT proved

- No physical-hardware evidence. Every semantic claim comes from the pinned Exodus source
  (quoted in t4) and from executed runs of the pinned build. The ADR therefore records **how the
  oracle behaves**, not a hardware measurement.
- The pinned headless build stalls the 68000 on a data-port read after a read command
  (t4 `oracle-notes.md` §3.1, ABERTO-1), and its `0x0C` low byte in the completed orderings
  disagreed with its own `0x00` read (ABERTO-2). The vectors are therefore requirements derived
  from the source, and the implementation must satisfy the source semantics; where the oracle is
  unusable, the test suite stands on its own assertions, and the ADR must not claim oracle
  agreement that was not observed.
- The exact hardware behaviour of a read while a write is pending (lock-up region) is left
  ABERTO by the pinned source and is out of scope.

## Decision request

Resolved 2026-11-24 by the owner direction recorded in the Status field: **Accept** and proceed
with the faithful Option B implementation (CFVDP003 + payload v8, golden re-freeze, bound
re-derivation, the §8 test plan). Options 2 (accept with changes) and 3 (reject) were not taken.

## Implementation status

Implemented and verified in the `agent/0c-vram-read-v8` worktree (no commit, push or merge):

- `Vdp` carries `read_buffer: u16`, `read_data_available` and `read_data_half_cached`, serialized
  in `CFVDP003`/version 3 (`SNAPSHOT_LEN` 65 811 -> 65 813) with all eight flag bits assigned and
  the existing validation order kept.
- `0x0C` reads the bit-inverted byte address `address ^ 1`, replaces only the low byte, keeps the
  buffer's upper byte, sets `read_data_available`, leaves `read_data_half_cached` clear, and the
  data-port read clears both flags and advances the address once. The refresh follows the pinned
  cache gate: it runs only while the write FIFO is empty (seeded from the next-read entry unless a
  16-bit read left a half-cached value); with a pending write the port returns the buffer's
  current value and a read neither consumes nor drains a FIFO entry.
- Payload `CFMD68K8`/8 with `CFMDSTATE8` and `CFMDCONF8`; v1-v7 are rejected by magic and by the
  version field. Goldens re-pinned with a recorded cause; `SNAPSHOT_PAYLOAD_BOUND` is
  206 277 (fresh 206 261, one pending 206 267 -> 206 269, two pending 206 277), derived, never
  widened. The unit, bus (68000 word/byte/long), Z80 portal (`0x7F00..=0x7F03`), snapshot
  round-trip, FIFO-seed, control-invalidation and snapshot-between-command-and-read vectors are
  in `crates/chronofork-megadrive/tests/`, `crates/chronofork-megadrive/src/bus.rs` and
  `crates/chronofork-cli/tests/0c_vram_read.rs`. The CLI binaries that print and pin the state
  identity (`crates/chronofork-cli/tests/debug.rs`, `demo.rs`) re-pin the moved values with a
  recorded cause. The dead reserved-bit mask check and `VdpError::SnapshotFlagsInvalid` are
  removed: `CFVDP003` assigns all eight flag bits. The doc comment on
  `CODE_VRAM_READ_8BIT` (`crates/chronofork-megadrive/src/vdp/regs.rs`) now describes the
  implemented behaviour. Evidence and gate logs are in `.research/0c-v8/impl/`.

## Evidence

- t4 decode: `.research/0c/oracle-notes.md` (PROVADO/INFERIDO/ABERTO, 13 vectors, source copies).
- t4 execution log: `.research/0c/commands.md` (commands, exit codes, ROM hashes, probe/trace
  outputs).
- Repo facts: `crates/chronofork-megadrive/src/vdp/mod.rs:157-218,514-…,1124-1137,1203-1441`,
  `vdp/regs.rs:86-100`, `backend.rs:151,176-241,1695-1757,1782,4092-4130`,
  `bus.rs:204-205,241,1950-2030`, `tests/vdp_ports.rs:213-249`,
  `tests/vdp_snapshot.rs:20,70-108,245-249`, `tests/snapshot_v4.rs:19-21`,
  `tests/snapshot_v5.rs:35-37`, `tests/audio_snapshot.rs:117-119`,
  `tests/cartridge_snapshot.rs:64-66`, `tests/snapshot_properties.rs:105`,
  `crates/chronofork-cli/tests/w18_payload_bound.rs`.
- Design elaboration: `.research/0c-v8/design.md`.
