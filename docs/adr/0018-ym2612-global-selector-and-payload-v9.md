# ADR-0018: YM2612 global bank+register selector and payload v9

- **Status:** **Accepted** — owner acceptance recorded **verbatim** on 2026-11-25
  (America/Fortaleza), selected in the ADR review:
  **"Aceito o ADR-0018 como está (Recomendado)"**. That acceptance authorises the
  decisions in this ADR and their implementation. It is **not** merge authorisation and
  does not authorise a push beyond the plan's PR step.
  Earlier plan-level direction (kept for the record, and superseded by the acceptance
  above): the owner approved the **staged plan** `chronofork-audio-latch-a-2026-11-25`
  through the pre-run review UI on `2026-11-25T14:44:37-03:00` (America/Fortaleza),
  fixing payload/state/config **v9**, YM snapshot **v2** and typed rejection of
  **v1–v8**; that record was the owner's direction **for the plan**, transmitted with
  the mechanism, the date and the plan name rather than as a quotation, and it stated
  that plan authorisation is **not** merge authorisation.
- **Date:** 2026-11-25.
- **Acceptance supplement (append-only, 2026-11-25):** the Status field above was changed
  from `Proposed` to `Accepted` only after the owner's verbatim text arrived. No other
  owner text is inferred or fabricated; the plan-level record is retained as history and
  is not restated in quotation marks.
- **Errata (append-only, 2026-11-25), correcting the accepted text without changing the
  decision:**
  - **E1 (R1).** §3 said the rejection order was `length -> magic -> version -> value` at a
    time when the first check was really the magic read, so the claim was false for an input
    with several defects. The accepted decision is unchanged; it is now **true**, because
    `restore_bytes` checks the exact length before it reads anything (short →
    `Truncated`, long → `TrailingBytes`), then magic, then version, then value. Pinned by
    `combined_defects_are_reported_length_first`.
  - **E2 (R2).** §2 described a `(YmBank, YmPort)` decode and a `write_port(port, value)`
    that took the bank from the port. The shipped split is `YmAccess`
    (`Address { bank, port }` / `Data { port }`), `write_address(bank, register)` for
    bank-bearing address writes and `write_port(YmPort::Data, value)` for data writes, with
    `write_port(YmPort::Address, ..)` a documented caller error that changes no state. No
    routing changed; only the description did.
  - **E3 (R3).** §3 carried two competing length formulas, one of which omitted the
    selector's own length. Both are replaced by the single shipped derivation
    `SNAPSHOT_SELECTOR_OFFSET + SNAPSHOT_SELECTOR_LEN + SNAPSHOT_TAIL_LEN` (521 + 2 + 333 =
    856), with the 521/333/856 compile-time assertions intact.
  - **E4 (N1, post-review, documentation only).** §2 named `ym_port` as the 68000 decoder
    that returns `YmAccess`. `ym_port` returns `Option<(YmBank, YmPort)>`; the wrapper that
    builds the `YmAccess` is `ym_access`. The name is corrected with no change to code,
    tests or behaviour, recorded by the round-2 review as a low nonblocking nit.
  - These errata invent no requirement and no new decision; the owner's verbatim acceptance
    above and the `Accepted` status are unaffected.
- **Milestone:** M8 (functional Mega Drive emulator) — YM2612 fidelity follow-up after W1.8.
- **Related:** [ADR-0002](0002-backend-boundaries.md) (backend boundaries),
  [ADR-0015](0015-verification-contract.md) (independent verification),
  [ADR-0016](0016-session-service-and-agent-api.md) (the byte/count quota preflight
  that consumes `snapshot_payload_bound`), and [ADR-0017](0017-vdp-read-cache-and-payload-v8.md)
  (the immediately preceding nested-semantic-change precedent: an inner snapshot bump
  **and** an outer payload bump in the same change). Supersedes no ADR; it extends the
  payload-versioning rule stated in [backend-model](../architecture/backend-model.md).
- **Input evidence (t1):** [`.research/audio-latch/spec-a.md`](../../.research/audio-latch/spec-a.md)
  (normative contract A1–A13, discriminator matrix M1–M11, not-proved list N1–N8), the
  decision brief [`.research/audio-latch/decision-brief.md`](../../.research/audio-latch/decision-brief.md),
  and the executed oracle probe
  [`.research/audio-latch/probe-oracle/summary.md`](../../.research/audio-latch/probe-oracle/summary.md)
  with its raw results [`.research/audio-latch/probe-oracle/results.json`](../../.research/audio-latch/probe-oracle/results.json).
  The pinned source is `ec0056151f3ff1c334d09ee375abb30a68fdfec5` (`exodus-headless`,
  branch `linux-headless`). The design elaboration is in
  [`.research/audio-latch/design-a.md`](../../.research/audio-latch/design-a.md).

## Context

The YM2612 exposes two port pairs: a **address** latch and a **data** latch per pair.
The slice models them as two independent per-bank latches
(`crates/chronofork-megadrive/src/audio/ym2612.rs:182`, `:208`), and the data port
selects the bank and then uses **that bank's** latch
(`crates/chronofork-megadrive/src/audio/ym2612.rs:240-246`):

```rust
pub fn write_bank(&mut self, bank: YmBank, port: YmPort, value: u8) {
    let index = bank.index();
    match port {
        YmPort::Address => self.address_latch[index] = value,
        YmPort::Data => self.write_register(bank, self.address_latch[index], value),
    }
}
```

The pinned oracle chooses a different model. `Devices/YM2612/YM2612.cpp:2339-2348`
overwrites **one** field `_currentReg` from either address port — `location == 0` stores
`data.GetData()` and `location == 2` stores `RegisterCountPerPart + data.GetData()` —
where `RegisterCountPerPart = 0x100` and `RegisterCountTotal = RegisterCountPerPart *
PartCount = 0x200` (`Devices/YM2612/IYM2612.h:23-25`). Both data ports (`location == 1 ||
location == 3`) then commit the register that selector names,
`Devices/YM2612/YM2612.cpp:2349-2356`. Both read ports return the status byte
unconditionally (`Devices/YM2612/YM2612.cpp:2299-2316`, `:2313`), and the
locking/latched-write guards sit only in the data branch (`:2352`, `:2494`).

The two models differ observably only when a program **interleaves** the port pairs. The
oracle probe measured the discriminating case on the executable pin: with timer A
programmed through the normal pair and left unstarted, the final write
`A0(27); D1(05)` through the **other** pair raised `status & 1` to **1** — identical to
the same-pair control — while the negative control stayed **0** and the Z80 mirror
`0x5F8B` behaved exactly like `0x4003`
(`.research/audio-latch/probe-oracle/results.json`; summarised in
[`summary.md`](../../.research/audio-latch/probe-oracle/summary.md)). Under the current
per-bank model that crossed write would have produced `0`. The slice's own probe measured
exactly that `0` (`.research/audio-latch/probe/src/main.rs:36-37`), so the divergence is
real, measured on both sides, and limited to the crossed/interleaved case.

Three facts constrain the change:

1. The register shadow is indexed by bank **and** address
   (`crates/chronofork-megadrive/src/audio/ym2612.rs:180-181`), and that part of the model
   is correct: the oracle keeps 512 registers (`Devices/YM2612/YM2612.cpp:130`,
   `Devices/YM2612/YM2612.h:475`). Only the **destination selection** is wrong.
2. The current YM snapshot is `CFYM2612`, version 1, 856 bytes
   (`crates/chronofork-megadrive/src/audio/ym2612.rs:67-71`), and it serialises **both**
   latches (`:735-737`, `:839-840`). Both latches are canonical state today.
3. The payload embeds that snapshot verbatim and every payload constant is fixed except
   the scheduler: `AUDIO_STATE_TAIL_LEN` contains `YM_SNAPSHOT_LEN`
   (`crates/chronofork-megadrive/src/backend.rs:197-205`), `FIXED_STATE_BODY_LEN` contains
   `AUDIO_STATE_TAIL_LEN` (`:215-231`), and
   `SNAPSHOT_PAYLOAD_BOUND = PAYLOAD_MAGIC.len() + 2 + FIXED_STATE_BODY_LEN +
   SCHEDULER_MAX_SNAPSHOT_LEN` (`:242-243`). Today those evaluate to
   `FIXED_STATE_BODY_LEN = 206_199`, fresh payload `206_261`, bound `206_277`
   (derivation in "Bound arithmetic" below, measured in the `audio-latch-a-ym2612`
   worktree at base `4993328340`).

## Problem

How should the YM2612 destination selection become the pinned model's single 9-bit
selector, without leaving per-bank false state in the canonical state, the public API or
the inspection surface — and how must the embedded snapshot and the outer payload be
versioned, validated and bounded so that no existing serialized byte is silently
reinterpreted?

## Decision

### 1. One canonical 9-bit selector replaces the two latches

`Ym2612` keeps its `registers: [[u8; 256]; 2]` file and replaces
`address_latch: [u8; 2]` with a single **`selector: u16`** holding `bank * 0x100 +
register`, i.e. `0x000..=0x1FF`. Derivation is total and explicit:

| Operation | Result |
|---|---|
| `bank = SelectorBank::One` | `selector < 0x100` (equivalently `selector & 0x100 == 0`) |
| `bank = SelectorBank::Two` | `selector >= 0x100` |
| `register` | `(selector & 0xFF) as u8`, and `registers[bank.index()][register]` is the target |

- **Reset value is `0`** (bank `One`, register `0x00`), matching the current zeroed latches
  and the oracle's `_currentReg = 0` on construction
  (`Devices/YM2612/YM2612.cpp:406`; the current init is
  `crates/chronofork-megadrive/src/audio/ym2612.rs:208`).
- **Every address-port write replaces the whole selector**: port 0 sets
  `selector = u16::from(value)` and port 2 sets `selector = 0x100 + u16::from(value)`.
  The bank is part of the selector, not of the port or of the previous value — this is the
  point the decision brief singles out at `decision-brief.md:28`.
- **Every data-port write commits to `registers[selector_bank][selector_register]`**, for
  either port.
- **No per-bank independent latch survives as canonical state.** The two-latch array is
  deleted, not retained "for compatibility": two independent latches cannot express which
  bank the last address write selected, which is precisely why no lossless migration from
  v1 exists (decision brief `:63`, `:72`, `:76`).
- **Restored out-of-range selectors are typed errors.** A deserialized value above
  `0x1FF` is not clamped and not masked; it is rejected (see §3, rejection order step 4).

`YmBank` itself is **retained** as the two-bank enumeration used for the register file and
for inspection (`crates/chronofork-megadrive/src/audio/ym2612.rs:93-117`), because the
model still has 2×256 registers and `YmBank::of_channel` is still meaningful. What is
removed is the implication that a **port** carries a bank.

### 2. Public API: the data port cannot name a bank

The current `write_bank(bank, port, value)` / `read_bank(bank, port)` pair is exactly the
false implication this ADR removes: a caller can pass `YmBank::Two` with an address latch
that names a bank-1 register. The shipped split is one entry point per access kind:

| Before | After | Why |
|---|---|---|
| `Ym2612::write(port, value)` | **unchanged** — `write(port, value)` | convenience form for the first port pair; it routes an `YmPort::Address` to `write_address(YmBank::One, value)` and an `YmPort::Data` to `write_port` |
| `Ym2612::read(port) -> u8` | **unchanged** — `read(port) -> u8` | both ports return status |
| `Ym2612::write_bank(bank, port, value)` | **removed**; a bank-bearing address access is `write_address(bank, register)` and a data access is `write_port(YmPort::Data, value)` | the bank belongs to the **address** write, because that write replaces the whole selector; a data write carries no bank at all |
| `Ym2612::read_bank(bank, port) -> u8` | **removed**; replaced by `read_port(port) -> u8` | reads ignore the port and the bank; both return `status()` |
| `Ym2612::register(bank, address) -> u8` | **unchanged** | it reads the 2×256 model file for tests and inspection; it selects nothing |
| `Ym2612::address_latch(bank) -> u8` | **removed**; replaced by `selector() -> u16`, `selector_bank()` and `selector_register()` | `u8` cannot carry a 9-bit selector and one value cannot be split into two |

`YmPort` keeps its two variants (`Address`, `Data`). `YmBank` keeps its two variants,
because the register file is still 2×256 and `YmBank::of_channel` is still meaningful.

**The bus decode is `YmAccess`, one type for both buses.** `ym_access` (68000) and
`z80_ym_access` (Z80) each return an `YmAccess`, which is either
`YmAccess::Address { bank, port }` — the access carries the bank because writing it replaces
the whole selector — or `YmAccess::Data { port }`, which carries **no bank**. A single
`apply_ym_access` dispatches: `Address` calls `write_address(bank, value)`, `Data` calls
`write_port(YmPort::Data, value)`. Because the bank exists only in the `Address` variant, no
caller
and no decode table can hand a bank to a data write, and the `0x5F8B` mirror is simply a
`Data` access that the `address & 3` decode lands on the second pair.

**Caller-error behaviour is explicit and does not affect routing.** `write_port` is a
data-port entry point: given `YmPort::Address` it changes no selector state and raises a
`debug_assert!` in debug builds, so a mistaken call cannot silently become a data write and
cannot move the destination of the next real data write. `write_address` is the only
bank-bearing entry point. The device, not the bus, decides the destination of a data write —
the bus never computes it — so no sound product routing was changed to fit the old prose.

Removing two public methods is a breaking change for this crate's public surface. There
were **no external callers** in the tree: `write_bank`/`read_bank` appeared only in
`crates/chronofork-megadrive/src/audio/ym2612.rs` and in the test helper at
`crates/chronofork-megadrive/tests/ym2612.rs:11-14`, which was converted (see §8).

### 3. YM snapshot `CFYM2612` version 2, length unchanged at 856

The magic stays `CFYM2612`; `SNAPSHOT_VERSION` becomes **2**. The selector sits at the same offset the two latches occupy today and occupies the same
two bytes, so `SNAPSHOT_LEN` stays **856** and no other offset moves. The complete v2
layout, in the serializer's own field order
(`crates/chronofork-megadrive/src/audio/ym2612.rs:728-766`):

| Offset | Length | Field |
|---|---|---|
| `0..8` | 8 | magic `CFYM2612` |
| `8` | 1 | version **2** |
| `9..265` | 256 | bank 1 register file (`registers[0]`) |
| `265..521` | 256 | bank 2 register file (`registers[1]`) |
| `521` | 1 | (**v1**) bank-1 address latch — superseded |
| `522` | 1 | (**v1**) bank-2 address latch — superseded |
| `521..523` | 2 | (**v2**) `selector`, little-endian `u16`, valid range `0x000..=0x1FF` |
| `523..595` | 72 | 24 envelopes, row-major `[channel][operator]`, 3 bytes each: `eg_state_code` `u8` + attenuation `u16` LE |
| `595..691` | 96 | 24 phases, `u32` LE |
| `691..739` | 48 | 12 feedback-history values (2 per channel), `u32` LE |
| `739..835` | 96 | 24 last outputs, `u32` LE |
| `835..837` | 2 | `lfo_phase`, `u16` LE |
| `837` | 1 | `status` |
| `838` | 1 | `irq_timer_a` flag |
| `839` | 1 | `irq_timer_b` flag |
| `840..848` | 8 | `timer_a_accumulator`, `u64` LE |
| `848..856` | 8 | `timer_b_accumulator`, `u64` LE |

The offsets sum to `856` exactly: the selector's two bytes plus a `333`-byte tail
(`72 + 96 + 48 + 96 + 2 + 1 + 1 + 1 + 8 + 8`). The shipped expression is the single
derivation

```text
SNAPSHOT_LEN = SNAPSHOT_SELECTOR_OFFSET + SNAPSHOT_SELECTOR_LEN + SNAPSHOT_TAIL_LEN
             = 521 + 2 + 333
             = 856
```

(mirroring `vdp/mod.rs:202-204`, which derives
`SNAPSHOT_LEN = SNAPSHOT_READ_BUFFER_OFFSET + 2`), with the compile-time assertions
pinning `SNAPSHOT_SELECTOR_OFFSET == 521`, `SNAPSHOT_TAIL_LEN == 333` and
`SNAPSHOT_LEN == 856`, so the layout cannot drift silently. There is **no** competing
`offset + tail` formula without the selector's own length: that form would give `854` and
is wrong. The implementation must **not** restate these numbers by hand where a constant
already exists.

Length is deliberately **not** changed: the task's working hypothesis is that the selector
fits in the two bytes the latches already used, and it does. Growing the format would be a
gratuitous second change in the same version bump and would move the payload length, the
bound and every golden for no gain.

Serialisation writes the selector as one `u16` LE; deserialisation reads it after the
version check. **The value is validated before any field is assigned** — `restore_bytes`
stages every field into locals and commits only after the whole input is validated
(`crates/chronofork-megadrive/src/audio/ym2612.rs`), so a rejected restore leaves the device
untouched.

**Rejection order** is fixed and is **length first**. The input's exact length is compared
before anything is read, so a combined-defect input is reported by its length alone:

1. **length** — an explicit exact-length pre-check compares `bytes.len()` against
   `SNAPSHOT_LEN` before the reader is constructed: a short input is
   `AudioSnapshotError::Truncated` and a long one is `AudioSnapshotError::TrailingBytes`,
   regardless of any bad magic, bad version or out-of-range selector the same input also
   carries. The reader's own partial reads and its final `finish()` tail check remain as
   defence in depth but are no longer the first line of defence;
2. **magic** — `AudioSnapshotError::Magic`;
3. **version** — `AudioSnapshotError::Version { found }`; a v1 payload has the exact length
   and a valid `CFYM2612` magic, so it is rejected **by version**, and the error names the
   version found;
4. **value** — `AudioSnapshotError::Value { field: "ym selector" }` when the `u16` exceeds
   `0x1FF`, and the existing envelope-state/attenuation validation afterwards. The variant
   already exists (`crates/chronofork-megadrive/src/audio/mod.rs:44-47`) and is used for
   envelope attenuation today; no new error type is introduced.

Combined-defect precedence is pinned by the test
`combined_defects_are_reported_length_first`
(`crates/chronofork-megadrive/tests/ym2612.rs`), which covers valid-v2-plus-extra-byte
(`TrailingBytes`), v1-plus-extra-byte (`TrailingBytes`, not `Version`), short-input-claiming-
v1 (`Truncated`, not `Version`), bad-magic-plus-extra-byte (`TrailingBytes`, not `Magic`),
bad-magic-plus-out-of-range-selector (`Magic`, not `Value`) and
v1-plus-out-of-range-selector (`Version`, not `Value`), each also asserting that the device
is not mutated.

There is **no silent reinterpretation**: no code path reads two v1 latch bytes as a v2
selector, and no padding/truncation leniency is added.

### 4. Payload, state and configuration v9

| Constant | v8 | v9 |
|---|---|---|
| `PAYLOAD_VERSION` | 8 | **9** |
| `PAYLOAD_MAGIC` | `CFMD68K8` | **`CFMD68K9`** |
| `STATE_MAGIC` | `CFMDSTATE8` | **`CFMDSTATE9`** |
| `CONFIG_MAGIC` | `CFMDCONF8` | **`CFMDCONF9`** |

(`crates/chronofork-megadrive/src/backend.rs:175-190`.)

`parse_payload` keeps its order — **magic first, then version**
(`crates/chronofork-megadrive/src/backend.rs:1754-1756`) — so v1–v8 payloads are rejected
by their own magic, and a v9 magic carrying an old version field is rejected by the
version check. Both surface as the existing typed `BackendError::InvalidSnapshot(String)`
with the existing message shapes ("Mega Drive payload magic mismatch", "Mega Drive payload
version {version}, expected {PAYLOAD_VERSION}").

**Why an inner-only bump is rejected.** An alternative considered was to bump only
`SNAPSHOT_VERSION` to 2 and leave the outer payload at v8, on the grounds that both
payloads are 206_xx bytes and the difference is two bytes deep. This is rejected for three
reasons:

1. **The outer version is the only consumer-visible identity.** A v8 payload has no YM
   snapshot magic or version in it that a reader can consult cheaply — the embedded bytes
   are an opaque 856-byte region as far as the payload parser is concerned
   (`backend.rs:1839` reads exactly `YM_SNAPSHOT_LEN`). A v8-tagged payload would carry v2
   YM bytes, and any consumer that trusts the outer tag (a session store, the service
   quota preflight at `backend.rs:1020-1024`, an older binary) would be told the layout is
   the one it knows.
2. **Equal length does not mean compatible.** The decision brief states this explicitly
   (`decision-brief.md:72`): two bytes that mean "two independent latches" cannot be
   re-read as "one global selector" without changing their meaning. The registry rule is
   to reject incompatible input rather than reinterpret it (`AGENTS.md`, rule 8).
3. **Precedent.** [ADR-0017](0017-vdp-read-cache-and-payload-v8.md) bumped **both** layers
   for a nested semantic change: `CFVDP002` → `CFVDP003` **and** payload v7 → v8, with the
   §5 table listing all four constants (`0017-vdp-read-cache-and-payload-v8.md:211-231`).
   An inner-only bump here would make the two nested-semantic-change ADRs disagree about
   the same question.

Consequences that follow and are accepted: the **state hash** changes because it is
`hash(STATE_MAGIC + state body)` (`backend.rs:1704-1709`) and `STATE_MAGIC` changed; the
**configuration hash** changes only because `CONFIG_MAGIC` changed — the configuration
inputs (ROM, video standard, TMSS, feature tag, SRAM) do not change, so **no
`CONFIG_FEATURE_TAG` bump is justified** (`backend.rs:245-249`).

### 5. Bound arithmetic: the bound may stay 206_277, and it is derived, not asserted

Every number below comes from a named constant or from a measurement taken in this
worktree. Nothing is hand-written into the format.

Named constants (all with their source):

| Constant | Value | Source |
|---|---|---|
| `AUDIO_STATE_TAIL_LEN` components | `Z80_SNAPSHOT_LEN` 43, Z80 RAM `0x2000` 8_192, carry 8, arbitration `2 + 2`, bank 8, `YM_SNAPSHOT_LEN` 856, `PSG_SNAPSHOT_LEN` 34, `MIXER_SNAPSHOT_LEN` 56 | `backend.rs:197-205`; `z80/mod.rs:72`, `bus.rs:169`, `audio/psg.rs:110`, `audio/mixer.rs:73`, `audio/ym2612.rs:71` |
| `FIXED_STATE_BODY_LEN` | 90 control bytes + `RAM_SIZE` 65_536 + `VDP_SNAPSHOT_LEN` 65_813 + 14 + 1 + `SRAM_SIZE` 65_536 + 2, plus `AUDIO_STATE_TAIL_LEN` | `backend.rs:215-231`; `bus.rs:104`, `vdp/mod.rs:204`, `cartridge.rs:41` |
| `SCHEDULER_MAX_SNAPSHOT_LEN` | 68 = header 52 (`8 + 2 + 1 + 8 + 2·8 + 2·8 + 1`) + 2 levels × `SCHEDULER_PENDING_SLOT_LEN` 8 | `interrupts.rs:90-107`; pinned by `backend.rs:4185-4186` |

Evaluating them:

```
AUDIO_STATE_TAIL_LEN = 43 + 8192 + 8 + 2 + 2 + 8 + 856 + 34 + 56 = 9201
FIXED_STATE_BODY_LEN = 197352 + 9201                            = 206199
fresh      = 8 + 2 + 206199 + 52  (scheduler zero-pending)      = 206261
one        = fresh + 8                                          = 206269
two        = fresh + 16                                         = 206277
BOUND      = 8 + 2 + FIXED_STATE_BODY_LEN + 68                  = 206277
```

**Hypothesis: the bound may stay 206_277 under model A — PROVEN.**

- The change replaces two `u8` write-only fields with one `u16`, i.e. **two bytes with two
  bytes** (`ym2612.rs:182` → new `selector: u16`). `SNAPSHOT_LEN` stays 856, so
  `AUDIO_STATE_TAIL_LEN`, `FIXED_STATE_BODY_LEN`, the fresh/one/two payload lengths and
  `SNAPSHOT_PAYLOAD_BOUND` are **numerically unchanged**.
- Independent confirmation of the pre-change values in this worktree (read-only probe):
  `PAYLOAD_VERSION = 8`, `SNAPSHOT_PAYLOAD_BOUND = 206277`, `YM_SNAPSHOT_LEN = 856`,
  `VDP_SNAPSHOT_LEN = 65813`, and the measured fresh/one-pending payload lengths
  `206261` / `206269` (delta 8), which agree with the derivation above.
- The tightness properties ADR-0017 §7 established therefore carry over to v9 **as
  numbers**: the two-pending state still reaches the bound exactly, the 0/8/16 progression
  is unchanged, and NTSC/PAL/imported-SRAM sweeps must still not exceed it
  (`backend.rs:4182-4362`).
- The **derivation identity** still has to be re-proved for v9, because
  `SNAPSHOT_PAYLOAD_BOUND` is an expression, not a literal (`backend.rs:242-243`). If
  implementation chooses any encoding that grows the format, the delta must be quantified
  (it would land in `FIXED_STATE_BODY_LEN`, and therefore in all three payload lengths and
  the bound) and justified; moving the number to make a gate green is forbidden.

### 6. Golden re-freeze with a recorded cause

| Golden | v8 value | v9 value | Cause |
|---|---|---|---|
| demo payload length (6 steps, `backend.rs:4095-4136`) | 206_261 (`:4113`) | **206_261 (unchanged)** | the format did not grow |
| demo payload hash | `32f4a4bb…` (`:4125`) | moves | **versioning**: the magic prefix and the `u16` version word changed |
| demo state hash | `acbe068c…` (`:4129`) | moves | **versioning**: `STATE_MAGIC` is the hash prefix |
| demo configuration hash | `f454b76b…` (`:4133`) | moves | **versioning**: `CONFIG_MAGIC` changed; the configuration did not |
| CLI `megadrive demo` stdout digest (`crates/chronofork-cli/tests/demo.rs:143-145`) | `1659642e…` | moves | **versioning**: the demo prints the state identity |

The six-step demo ROM performs only **paired** writes through the same port pair, so no
step's canonical state changes under model A: every moved digest above is attributable to
the identity change and none to behaviour. That split must be recorded in the test
comments and in the implementation report (decision brief `:75`). No golden may be
deleted, skipped or weakened; an **unchanged** golden that starts failing is a bug.

### 7. Locking and latched writes: not added, and why the crossed write is still correct

The pinned oracle guards every committed data write with
`!_rawRegisterLocking[_currentReg] && !CheckForLatchedWrite(...)`
(`Devices/YM2612/YM2612.cpp:2352`, `:2494`). **This slice has no locking model at all**
(`spec-a.md` §2.4), and **model A does not add one.** Consequences:

- For an **unlocked** register — every register this slice implements — the guard is
  identically true, so the committed destination is exactly the selector the last address
  port wrote. The crossed-write outcome measured on the oracle (crossed = 1, mirror = 1)
  and the matrix in §8 are therefore correct *for the registers the slice models*, without
  any locking state.
- Locking, `CheckForLatchedWrite`, register trails and out-of-order access rollback
  (`Devices/YM2612/YM2612.h:394-395`, `YM2612.cpp:2325-2332`) stay in the **honest-limits**
  list (see "What is NOT proved"): no hardware or oracle measurement covers them, and the
  pinned MCP surface does not even publish the register file
  (`decision-brief.md:170-181`).
- Adding locking is explicitly **out of scope** for this ADR. If it is ever added, it needs
  its own contract, its own snapshot fields and its own evidence.

### 8. Test matrix

Device-level and bus-level vectors, all of which must agree across the three access paths
(device API, 68000 `0xA04000..=0xA04003`, Z80 `0x4000..=0x4003` + mirror `0x5F8B`):

| # | Vector | Expected under A |
|---|---|---|
| V1 | `A0(30); A1(40); D0(11)` | `bank1[40] = 11`, `bank0[30] = 0` |
| V2 | `A1(40); A0(30); D1(22)` | `bank0[30] = 22`, `bank1[40] = 0` |
| V3 | `A0(30); A1(30); D0(11)` | `bank1[30] = 11`, `bank0[30] = 0` (kills "shared address, bank from port") |
| V4 | `A0(30); D0(11)` and `A1(40); D1(22)` | positive controls |
| V5 | timer A via `A0(24); D0(FF); A0(25); D0(03)`, then `A0(27); D0(05)` | `status & 1 = 1` |
| V6 | as V5 but `D1(05)` through `0x4003`/`0xA04003` | `status & 1 = 1` (oracle-measured) |
| V7 | as V6 but through the mirror `0x5F8B` | `status & 1 = 1` (oracle-measured) |
| V8 | negative control: no start, `A0(27); D0(00)` | `status & 1 = 0` |
| V9 | `A0(30); A1(40)`; snapshot; restore; `D0(11)` | identical bytes to the no-restore path, `bank1[40] = 11` |
| V10 | `A1(40); A0(30)`; snapshot; restore; `D1(22)` | identical bytes to the no-restore path, `bank0[30] = 22` |
| V11 | crossed write `A0(40); A1(11)`; `D0(22); D1(33)` | `bank2[0x11] = 22`, `bank1[0x40] = 33` |
| V12 | `A0(30); A1(40)`; `state_hash` / inspection | the selector after `A0(30)` differs from the selector after `A1(40)` |

Plus: mirror-period sweep over `0x4000..=0x5FFF` asserting period 4; status identical on
all four logical ports and the mirror; every one of the four ports typechecks against the
`write_port`/`read_port` API.

Negative / mutation controls (each must fail if the mutation is applied):

| # | Mutation | Caught by |
|---|---|---|
| N1 | restore the per-bank latch in the data branch | V1, V2, V3, V9–V11 |
| N2 | keep a shared 8-bit address and derive the bank from the data port | V3, V11 |
| N3 | address port writes only the 8 bits | V1 |
| N4 | restore does not restore the selector | V9, V10, V12 |
| N5 | `SNAPSHOT_VERSION` stays 1, or a v1 payload is accepted | version-rejection test |
| N6 | `PAYLOAD_VERSION` stays 8, or a v8 payload is accepted | payload-rejection test |
| N7 | out-of-range selector accepted (clamped instead of rejected) | value-rejection test |
| N8 | bound written as a literal instead of derived | bound-tightness test |

Rejection tests: payload v1–v8 by **both** magic and version word (extending
`backend.rs:3389-3447` and `crates/chronofork-megadrive/tests/audio_snapshot.rs:220-266`),
`CFYM2612` with version 1 rejected by version, truncated and oversized YM payloads
rejected by length **before** magic/version/value are examined, selector `0x200`/`0xFFFF`
rejected by value with the machine unchanged, and combined-defect inputs proving the
length-first precedence (`combined_defects_are_reported_length_first`).
Bound tests: `the_payload_bound_is_the_two_pending_payload` (`backend.rs:4182`),
`sweep_stays_within_payload_bound` (`:4271`),
`the_payload_bound_holds_for_ntsc_pal_and_imported_sram` (`:4309`), and the service+CLI
regression `crates/chronofork-cli/tests/w18_payload_bound.rs`. Inspection test
`inspect_exposes_z80_arbitration_and_audio_pseudo_registers`
(`crates/chronofork-megadrive/tests/audio_snapshot.rs:344-390`) must replace
`ym.address_latch0`/`ym.address_latch1` with `ym.selector` and adjust its register count.

### 9. Inspection replacement

`ym.address_latch0` and `ym.address_latch1` (`backend.rs:1275-1280`) must **not** survive:
they publish two independent per-bank latches, which is false state under model A. They are
replaced by a single entry:

| Key | Value | Meaning |
|---|---|---|
| `ym.selector` | `u64::from(selector)`, `0x000..=0x1FF` | the global bank+register selector; `bank = value >= 0x100`, `register = value & 0xFF` |

The register count of the inspection map decreases by one. The getter is
`Ym2612::selector() -> u16`; `address_latch(bank)` is deleted rather than left as a
misleading alias.

## Consequences

**Positive:** the slice follows the pinned selector model, including the bank in the
selection and the equivalence of the two data ports; the crossed-write timer case matches
the oracle measurement instead of contradicting it; the canonical state, the public API and
the inspection surface stop carrying two independent per-bank latches; the embedded
snapshot becomes a 9-bit selector with a typed out-of-range rejection; the outer identity
moves with the inner semantics so no consumer can mis-read the layout; and the payload
length and bound are unchanged, so the change does not silently widen the per-session quota
that ADR-0016 consumes.

**Negative:** a format bump in two layers, so every v1 YM snapshot and every v1–v8 payload
becomes invalid input and must be re-created or re-run; four golden digests move for
versioning reasons; `write_bank`/`read_bank`/`address_latch` are removed from the public
surface (no external callers, one test helper to convert); and the inspection map changes
shape. Any accepted document that states payload v8 for this backend needs a one-line
follow-up outside this ADR's scope.

**Neutral/known:** the selector is not host-observable except through a checkpoint, the
`ym.selector` inspection entry and the state hash — exactly like the latches today. The
register shadow layout, the audio output and the mixer are untouched, so determinism and
the audio pipeline are unaffected apart from the intended routing change.

## Alternatives considered and discarded

- **A. Keep the per-bank latches and add a global-selector mode.** A switch would double
  the canonical state, the inspection surface and the test matrix, and would let two
  different machines share one identity. The decision brief rejects inventing a selectable
  mode without demonstrated need (`decision-brief.md:66`).
- **B. Share only the 8-bit register number and keep the bank from the data port.**
  Measured-against by V3 and V11: `A0(30); A1(30); D0(11)` would still write `bank0[30]`,
  and a crossed write would still target the wrong bank. This is the variant the decision
  brief names at `:28` and `:86`.
- **C. Keep `write_bank(bank, port, value)` but validate that `bank` matches the selector
  for data ports.** Rejected: it leaves the false parameter in the signature and makes the
  caller responsible for a consistency the model can express by construction. Removing the
  parameter is the same amount of work at the call sites and removes the failure mode.
- **D. Bump only the inner YM version to 2 and leave the payload at v8.** Rejected in §4:
  the outer tag is the only cheap consumer-visible identity, equal length is not
  compatibility, and ADR-0017 bumped both layers for the same class of change.
- **E. Encode the selector as two bytes in a new trailing field so the old latch offsets
  keep their v1 meaning.** Rejected: `SNAPSHOT_LEN` would grow by 2, moving
  `FIXED_STATE_BODY_LEN`, all three payload lengths and the bound for no semantic gain; it
  also invites a "read the old bytes and the new bytes" leniency that the versioning rule
  forbids.
- **F. Migrate v1 snapshots by guessing which latch was written last.** Rejected: that
  information does not exist in the v1 bytes (decision brief `:63`, `:76`); a migration
  would invent state. Rejection and re-creation is the only honest path.
- **G. Model locking at the same time.** Rejected: no evidence exists for it in this slice
  or on the observable oracle surface, and it would enlarge the snapshot again. Out of
  scope, recorded in the limits (§7).

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| A golden is "fixed" by weakening the assertion | §6: re-pin with a recorded cause; review checks no assertion is deleted or relaxed |
| The bound is enlarged instead of re-derived | §5: the identity is an expression; tightness must still reach 206_277 exactly; a failure is reported, not accommodated |
| A v1 YM snapshot is accepted by a lenient parser | §3: the exact-length pre-check runs first, then magic, then version, so a v1 payload is rejected by version; explicit tests cover version 1, bad length and out-of-range value, and combined-defect precedence |
| A v8 payload carrying v2 YM bytes is accepted | §4: the outer magic moves and v1–v8 are exercised by both magic and version |
| The inspection surface keeps publishing per-bank state | §9: the two keys are replaced, not aliased, and the inspection test asserts the new key |
| Callers of the removed API silently keep compiling | §2: the parameter is removed, not defaulted; the compiler finds every call site |
| The crossed-write change is claimed as a sound or compatibility improvement | "What is NOT proved": the change restores pin parity for routing only |

## What is NOT proved

- **Pin parity only.** Every semantic claim comes from the pinned source
  (`ec005615…`) and from executed runs of the pinned `exodus-probe` binary
  (`sha256 07c56c54…`, `.research/audio-latch/probe-oracle/identity.json`). No
  physical-hardware measurement exists, and the pin's own author attributes the data-port
  equivalence to hardware tests he does not document
  (`Devices/YM2612/YM2612.cpp:2334-2338`; `spec-a.md` C6).
- **Not sound equivalence.** No PCM or audio sample was compared with the oracle. Internal
  PCM hashes prove determinism of a fixture, not agreement
  (`decision-brief.md:36`).
- **Not game compatibility.** No commercial ROM was executed and no compatibility gain or
  regression was measured (`decision-brief.md:126`).
- **Not all registers.** The executable oracle probe observes only the **global** timer
  register `0x27` through `status & 1`. The routing of non-global registers follows the
  pinned source and must be reported as source-derived until a register-observing probe
  exists (`decision-brief.md:177-181`; `spec-a.md` N1).
- **No locking, no write timing, no out-of-order rollback.** This ADR adds none of them and
  claims none (§7; `spec-a.md` N2, N3).
- **The 9-bit width is algebraic.** `RegisterCountPerPart * PartCount = 0x200` and the
  selector's maximum value is `0x1FF`; nothing here measures the chip's internal bus
  (`spec-a.md` N7).

## Decision request

Resolved 2026-11-25 by the owner acceptance recorded in the Status field: **"Aceito o
ADR-0018 como está (Recomendado)"** — accept the single 9-bit selector (§1), the API change
that removes the bank from the data path (§2), `CFYM2612` version 2 at length 856 with the
exact-length-first rejection order of §3, payload/state/config v9 with the inner-only alternative rejected
(§4), the derived bound held at 206_277 (§5), the golden re-freeze policy (§6), locking out
of scope (§7), the §8 test matrix and the inspection replacement (§9). The acceptance
authorises implementation; it is not merge authorisation.

## Implementation status

Implemented and verified in the `agent/audio-latch-a-ym2612` worktree (no commit, push or
merge) after the owner acceptance above:

- `Ym2612` carries one `selector: u16` (`0x000..=0x1FF`) instead of `address_latch: [u8; 2]`;
  `write_address(bank, register)` replaces the whole selector and `write_port(YmPort::Data,
  value)` commits to `registers[selector_bank()][selector_register()]`. `write_bank` and
  `read_bank` are removed (they let a caller name a bank on the data path), `read_port`
  replaces `read_bank`, and `address_latch(bank) -> u8` is replaced by `selector() -> u16`
  plus `selector_bank()`/`selector_register()`. The 68000 and Z80 decodes both produce one
  `YmAccess` type, so no data path can carry a bank and the `0x5F8B` mirror shares the
  bank-2 data port with `0x4003`.
- YM snapshot `CFYM2612` version 2 at the unchanged length 856, with the selector at
  `521..523` (`SNAPSHOT_SELECTOR_OFFSET`), the length derived as
  `SNAPSHOT_SELECTOR_OFFSET + SNAPSHOT_SELECTOR_LEN + SNAPSHOT_TAIL_LEN` and pinned by
  compile-time assertions. Rejection order is **length first** as an explicit exact-length
  pre-check of `bytes.len()` against `SNAPSHOT_LEN` before the reader is constructed, then
  magic, then version, then value; version 1 is rejected by version and a selector above
  `0x1FF` by `AudioSnapshotError::Value { field: "ym selector" }`, with the device
  untouched.
- Payload/state/config v9 (`CFMD68K9`/`CFMDSTATE9`/`CFMDCONF9`); v1-v8 rejected by magic
  and by version, with the rejection tests extended to v8.
- Inspection: the two `ym.address_latch0`/`ym.address_latch1` entries are replaced by one
  `ym.selector`.
- Bound: `FIXED_STATE_BODY_LEN` 206_199 and `SNAPSHOT_PAYLOAD_BOUND` 206_277 are unchanged,
  and the arithmetic is now asserted explicitly in
  `the_payload_bound_is_the_two_pending_payload` so a format growth fails there rather than
  moving the number.
- Goldens re-frozen with the cause recorded: the payload hash, state hash, configuration
  hash, the six CLI `debug` state hashes and the `megadrive demo` digest all moved for
  **versioning**; the payload length is still 206_261 and the demo narration is still 1994
  bytes, because the demo performs only paired YM writes through the same port pair.
- Tests added: selector derivation/reset, forward and reverse crossed writes, the
  same-register-number vector, paired-write controls, crossed key-on, all three access
  paths plus the mirror, the period-4 `SOUND` sweep, status parity on every port, the
  crossed timer discriminator with same-pair and no-start controls, the
  snapshot-between-address-and-data continuation, the `CFYM2612` v2 identity, version-1
  rejection without mutation, invalid selector/length rejections, and the combined-defect
  precedence test `combined_defects_are_reported_length_first` (six defect pairs, each also
  asserting no mutation).
- No test was deleted, no `#[ignore]` or `#[allow]` was added and no lint was relaxed. The
  existing per-bank test `address_latches_are_separate_per_bank` was **converted** to
  `one_global_selector_names_the_destination_of_both_data_ports`, which asserts the
  converted calls' new and correct outcome.

## Evidence

- Normative evidence and citations: `.research/audio-latch/spec-a.md` §1 (pin), §2 (slice),
  §3 (contract A1–A13, limits N1–N8), §4 (matrix M1–M11 with measured/predicted labels).
- Oracle probe: `.research/audio-latch/probe-oracle/{summary.md,results.json,identity.json,validation.txt}`
  (positive 1, negative 0, crossed 1, mirror 1; 8 exits 0; clone and binary hashes verified;
  "No physical-hardware or audio-sample equivalence conclusion").
- Slice probe: `.research/audio-latch/probe/src/main.rs` and
  `.research/audio-latch/probe-command.txt` (forward/reverse interleave, timer crossed = 0,
  snapshot-after-both-addresses byte-identical restore).
- Repo facts quoted above: `crates/chronofork-megadrive/src/audio/ym2612.rs:67-71,93-117,179-246,265-269,727-778,787-849`;
  `crates/chronofork-megadrive/src/audio/mod.rs:31-48`; `crates/chronofork-megadrive/src/bus.rs:104,160-196,169,776,800,1137,1318-1322,1779-1817,1958-1961,2006-2022`;
  `crates/chronofork-megadrive/src/backend.rs:175-249,1020-1024,1275-1280,1639-1709,1754-1756,1839,3389-3447,4095-4136,4182-4362`;
  `crates/chronofork-megadrive/src/interrupts.rs:90-107`; `crates/chronofork-megadrive/src/vdp/mod.rs:202-204`;
  `crates/chronofork-megadrive/src/cartridge.rs:41`; `crates/chronofork-megadrive/src/z80/mod.rs:72`.
- Tests quoted above: `crates/chronofork-megadrive/tests/ym2612.rs:11-14,54-62,188`;
  `crates/chronofork-megadrive/tests/audio_snapshot.rs:116-119,220-266,268-342,344-390`;
  `crates/chronofork-cli/tests/demo.rs:143-145`; `crates/chronofork-cli/tests/0c_vram_read.rs:162-169`.
- Bound measurements taken in the `audio-latch-a-ym2612` worktree at base `4993328340`
  with a read-only probe (`CARGO_TARGET_DIR=/tmp/audio-latch-bound-target`):
  `PAYLOAD_VERSION=8`, `SNAPSHOT_PAYLOAD_BOUND=206277`, `YM_SNAPSHOT_LEN=856`,
  `VDP_SNAPSHOT_LEN=65813`, fresh payload `206261`, one-pending `206269`,
  `FIXED_STATE_BODY_LEN=206199`, `AUDIO_STATE_TAIL_LEN=9201`.
- Design elaboration: `.research/audio-latch/design-a.md`.
