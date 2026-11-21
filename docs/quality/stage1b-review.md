# Stage 1b Independent Review — Mega Drive VDP + Interrupt Integration

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** Complete (review only; no implementation files edited)
- **Reviewer:** `verifier` (independent reviewer, not an author of these changes)
- **Date:** 2026-11-14
- **Reviewed tree:** commit `73e0bfa` plus the uncommitted stage-1b working tree
  (`crates/chronofork-megadrive/src/vdp.rs` and
  `crates/chronofork-megadrive/src/interrupts.rs` are new/untracked)
- **Verdict:** **Needs revision** — two medium findings (a 68000 decoding defect and
  stale authoritative documentation) plus three low findings. The executable gate is
  green and the integration architecture is sound; the review does not pass until
  the findings below are addressed.

## 1. Scope and method

This is a read-only review of the integrated slice described by EPIC-0002:
the expanded 68000 subset, the VDP status/control/VRAM slice, the deterministic
VBlank/HBlank scheduler, and their integration into the real backend, snapshots,
sessions and CLI. I inspected the diffs against `73e0bfa`, read every file in
`crates/chronofork-megadrive/src/` and the CLI, compared the durable documentation
and licensing records against the code, and executed the requested verification
commands. I did not edit code, tests or any document other than this review.

## 2. Commands and results

| Command | Result |
|---|---|
| `./scripts/check.sh` | **Passed** (exit 0): `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` |
| `cargo test --workspace` (inside the gate) | **96 passed, 0 failed**: core 8, sessions 4 + acceptance 5, test-backend 7, megadrive 69, CLI 3 |
| `cargo run -p chronofork-cli -- megadrive demo` | **Passed** (exit 0), final line `chronofork megadrive demo: PASS` |
| `git status --short --ignored` | Only the expected stage-1b modifications/untracked modules; only `target/`, `.agent-teams/`, `.agents/`, `.planning/`, `.research/` ignored |
| Markdown relative-link audit (33 tracked `*.md`, 75 relative links) | **0 broken links** |

The demo output observed in this session (abridged to the evidence lines):

```text
chronofork megadrive demo: 68000 + VDP port + interrupt scheduler slice
vdp: register 15 (auto-increment)=2 status=0x0100 address=0x0002
vdp: program wrote and read back VRAM word 0x1234 through the data port
interrupt: HBlank pending at tick 64 (level 4, vector 0x70, mask 7)
interrupt: HBlank delivered; handler pc=0x2C2 D7=1
interrupt: VBlank pending level 6 vector 0x78 at tick 896 (quantum=4); snapshot restored it
snapshot: VDP, scheduler and CPU state reproduced from the checkpoint
chronofork megadrive demo: PASS
```

## 3. Verification matrix

| # | Review criterion | Result | Evidence |
|---|---|---|---|
| 1 | No upstream code or ROMs | **Pass** | No tracked binary payloads (`git ls-files` contains only 33 `*.md`, 26 `*.rs`, 9 `*.toml`, 1 `*.sh`, 1 `*.lock`, `.gitignore`, `.editorconfig`, `LICENSE`); no ROM extensions tracked; [provenance](../licensing/provenance.md) records "Incorporated upstream code: **None**"; [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md) lists only RustCrypto crates; `.research/` is ignored and not referenced by any manifest; no upstream copyright headers appear in the crate sources |
| 2 | File/API contracts | **Partial** | `Vdp`, `InterruptScheduler`, `MegadriveBus` and the backend payload/state/config formats match the t3/t4/t5 contracts; the 68000 `LEA`/`PEA` decoder does not match its documented contract (F1) |
| 3 | Snapshot version bump and compatibility rejection | **Pass** | Payload magic/version bumped to `CFMD68K2`/2 ([backend.rs:42](../../crates/chronofork-megadrive/src/backend.rs#L42), [:48](../../crates/chronofork-megadrive/src/backend.rs#L48)); `parse_payload` rejects any other version ([:515](../../crates/chronofork-megadrive/src/backend.rs#L515)); old/malformed payload rejection is tested ([:887](../../crates/chronofork-megadrive/src/backend.rs#L887)). The outer envelope `BACKEND_SNAPSHOT_FORMAT_VERSION` intentionally stays 1 ([backend-api snapshot.rs:12](../../crates/chronofork-backend-api/src/snapshot.rs#L12)); old checkpoints are still stopped by the config hash and payload version, see §5 |
| 4 | VDP/interrupt state included in hashes | **Pass** | `state_hash()` hashes the VDP snapshot and scheduler snapshot ([backend.rs:209](../../crates/chronofork-megadrive/src/backend.rs#L209)); `state_hash_for` writes both blobs into the digest ([backend.rs:465](../../crates/chronofork-megadrive/src/backend.rs#L465)); `config_hash` includes ROM + scheduler standard + instructions-per-line ([backend.rs:440](../../crates/chronofork-megadrive/src/backend.rs#L440)). Snapshot payload inclusion is asserted by `snapshots_include_vdp_and_scheduler_state` ([:786](../../crates/chronofork-megadrive/src/backend.rs#L786)); deterministic convergence including devices is asserted by `two_machines_converge_including_devices` ([:1004](../../crates/chronofork-megadrive/src/backend.rs#L1004)) |
| 5 | Fork/restore isolation | **Pass** | Restore validates identity/version/config/payload/hash and builds a cloned bus + scheduler replacement before committing ([backend.rs:299](../../crates/chronofork-megadrive/src/backend.rs#L299), [:347](../../crates/chronofork-megadrive/src/backend.rs#L347)); forks restore into a fresh backend (sessions `fork_from_snapshot`); tests cover source isolation with pending VBlank/HBlank and VDP state ([:945](../../crates/chronofork-megadrive/src/backend.rs#L945), [:972](../../crates/chronofork-megadrive/src/backend.rs#L972)); the CLI demo independently shows fork/source divergence and restore |
| 6 | Deterministic behavior | **Pass** | No host time, randomness, `HashMap` iteration or thread scheduling in the slice (`grep` found none of `SystemTime`, `Instant`, `rand`, `HashMap`); scheduler event generation is step-size independent (`interrupts::tests::event_generation_is_independent_of_step_size`); two independent machines converge byte-for-byte including VDP and scheduler snapshots |
| 7 | Capabilities do not overclaim rendering/audio/tracing | **Pass** | Factory advertises only execution, save states, memory read/write, register inspection, instruction stepping, cloning and deterministic replay; `rendering`, `audio`, `breakpoints`, `watchpoints`, `tracing` remain `false` ([backend.rs:95](../../crates/chronofork-megadrive/src/backend.rs#L95)). Descriptor and CLI text state "no rendering/audio/Z80/DMA/controllers or cycle-accurate timing" |
| 8 | Docs match code | **Fail** | The updated slice documents match the code, but several authoritative/architecture documents still say VDP and interrupts are entirely unimplemented, and three documents still quote the old 40-test count: F2, F3, F4, F5 |
| 9 | Full gate passes | **Pass** | `./scripts/check.sh` exit 0; see §2 |
| 10 | Markdown relative-link audit | **Pass** | 75 relative links across 33 tracked Markdown files, 0 broken |

## 4. Findings

### F1 — Medium — `LEA`/`PEA` decode the absolute-*word* encodings as absolute-long

- **Files:** [cpu.rs:327](../../crates/chronofork-megadrive/src/cpu.rs#L327),
  [cpu.rs:335](../../crates/chronofork-megadrive/src/cpu.rs#L335),
  tests at [cpu.rs:851](../../crates/chronofork-megadrive/src/cpu.rs#L851) and
  [cpu.rs:862](../../crates/chronofork-megadrive/src/cpu.rs#L862)
- **Problem:** The decoders use `(opcode & 0xF1F8) == 0x41F8` and
  `(opcode & 0xFFF8) == 0x4878`, then always fetch a 32-bit address. In the 68000
  encoding, `0x41F8`/`0x4878` are the absolute-**word** forms; the absolute-long
  forms claimed by the docs are `0x41F9`/`0x4879`. Because the masks ignore the
  low EA-register bits, `LEA (xxx).W,An`, `LEA (d16,PC),An` and `PEA (xxx).W`/
  `PEA (d16,PC)` are silently executed as absolute-long instead of being rejected,
  and on real instruction encodings they consume the wrong number of bytes. The
  two "abs_long" unit tests use `0x47F8`/`0x4878` (absolute word) as their fixtures,
  so they enshrine the defect and do not test the advertised form. The documented
  form itself (`0x41F9`/`0x4879`) works, but the module promise that "unsupported
  opcodes ... are never silently skipped" does not hold for the variants listed
  above. This is a real 68000 decoding defect in a claimed instruction.
- **Required fix:** Match the absolute-long forms precisely —
  `(opcode & 0xF1FF) == 0x41F9` for `LEA` (bits 11–9 are the destination register)
  and `opcode == 0x4879` for `PEA` — and update the tests to use `0x47F9`/`0x4879`.
  Add negative tests asserting that `0x41F8`/`0x4878` are rejected as illegal, and
  keep [megadrive-slice.md:56](../architecture/megadrive-slice.md#L56) accurate.

### F2 — Medium — Authoritative docs still state that VDP and interrupts are wholly unimplemented

- **Files:** [AGENTS.md:16](../../AGENTS.md#L16) (and the status paragraph above it),
  [overview.md:132](../architecture/overview.md#L132),
  [backend-model.md:118](../architecture/backend-model.md#L118),
  [execution-model.md:105](../architecture/execution-model.md#L105) and
  [execution-model.md:107](../architecture/execution-model.md#L107)
- **Problem:** `AGENTS.md` says "VDP, audio, Z80, interrupts, controllers and most
  68000 instructions are not implemented"; the architecture overview table says
  "VDP, audio, Z80, interrupts | Not implemented"; the backend-model page says the
  slice has "no VDP, audio, Z80, interrupts"; the execution-model known-limitations
  list says "No real hardware timing, DMA, interrupts or audio" and "State hash
  covers only synthetic state". All of these contradict the current working tree,
  which maps VDP ports `0xC00000..=0xC0000F`, drives VBlank status, schedules
  VBlank/HBlank, and includes both devices in the real backend state hash. The
  updated pages ([README](../../README.md), [docs/README](../README.md),
  [megadrive-slice.md](../architecture/megadrive-slice.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)) are accurate, so the project
  currently contradicts itself. `AGENTS.md` is the authoritative onboarding file;
  this is not a cosmetic stale count.
- **Required fix:** Update the status/implemented tables and the execution-model
  limitations to describe the partial VDP port/status/VRAM window, the
  instruction-quantum VBlank/HBlank scheduler, the device-inclusive real-backend
  hash, and the explicit remaining gaps (rendering, DMA, VINT-pending, `RTE`, SR mask
  gating, audio, Z80, cycle accuracy).

### F3 — Low — Stale "40 tests" counts in durable documents

- **Files:** [0001-rust-workspace.md:70](../adr/0001-rust-workspace.md#L70),
  [testing-strategy.md:552](testing-strategy.md#L552),
  [licensing-assessment.md:48](../research/licensing-assessment.md#L48)
- **Problem:** The first two are durable documents and now state the wrong current
  test count (40); the executable gate currently reports 96. The licensing
  assessment row quotes the old README as evidence and should at least be marked as
  a dated audit rather than current.
- **Required fix:** Update the ADR/testing-strategy counts (or make them relative to
  a dated run) and annotate the research row as of its audit date.

### F4 — Low — The "demo output" in quality-gates is not the exact output

- **File:** [quality-gates.md:82](quality-gates.md#L82)
- **Problem:** The block is introduced as "The `megadrive demo` output was" but the
  fork-isolation line is abbreviated to `fork isolation: fork=... source=...`. The
  rest of the output is reproduced verbatim. Evidence records should either be exact
  or explicitly labelled as an excerpt/redaction.
- **Required fix:** Paste the full hash values (they are deterministic) or label the
  block "excerpt; hashes shortened".

### F5 — Low — VDP half-completed command documentation does not match the write path

- **Files:** [vdp.rs:458](../../crates/chronofork-megadrive/src/vdp.rs#L458),
  [megadrive-slice.md:91](../architecture/megadrive-slice.md#L91)
- **Problem:** The architecture page says half-completed commands are rejected as
  typed errors, and the read path does reject a data read after only the first
  control word (`code = 0x10` → `UnsupportedTarget`). The write path only checks
  `code & 0x0F == VRAM_WRITE`, so a data write after only the first control word
  (`code = 0x11`) is accepted and writes VRAM. The unit test
  `data_port_rejects_unsupported_command_targets` covers only the read case. The
  behaviour may be defensible if `CD4` is treated as a read-only inhibit bit, but
  the documentation and tests currently disagree with it.
- **Required fix:** Choose one behaviour and make code, docs and tests agree —
  either reject data accesses while the command half is pending, or narrow the doc
  to "half-completed **read** commands" and add a write-path test that pins the
  intended semantics.

## 5. Acknowledged limitations (verified; not findings)

These are explicitly documented and fall outside the implemented slice, so they do
not count as overclaims:

- **SR mask does not gate delivery.** The backend delivers the highest-priority
  pending request before each instruction regardless of the 68000 mask
  ([backend.rs:12-19](../../crates/chronofork-megadrive/src/backend.rs#L12),
  [backend.rs:255](../../crates/chronofork-megadrive/src/backend.rs#L255)). The
  limitation, and the reason (the reset mask is 7 and no implemented instruction can
  change it), are stated in the module docs and in
  [megadrive-slice.md §Known limitations](../architecture/megadrive-slice.md#L148).
  `next_deliverable` implements the correct priority rule but intentionally is not
  wired in yet. The CLI prints `mask 7` next to the delivered HBlank, which makes the
  limitation visible rather than hidden.
- **No `RTE`.** Handlers cannot return; the synthetic handlers loop and each
  delivery pushes six bytes, so a very long run eventually produces a typed bus
  error instead of corrupting state (documented).
- **VINT-pending status bit is not wired.** Only VBlank and FIFO-empty can change
  in the VDP status register; other bits are documented constants
  ([vdp.rs:74-84](../../crates/chronofork-megadrive/src/vdp.rs#L74)).
- **Instruction-quantum timing.** One scheduler tick is one executed instruction,
  not one 68000 cycle; nominal NTSC/PAL line constants are model parameters
  ([interrupts.rs:1-45](../../crates/chronofork-megadrive/src/interrupts.rs#L1)).
- **`BACKEND_SNAPSHOT_FORMAT_VERSION` remains 1.** This is the generic backend
  envelope version; backward rejection is enforced by the backend-owned payload
  magic/version plus the changed config hash. Old payloads are rejected by tests and
  the acceptance criterion "old payload versions are rejected" is met. The coupling
  is deliberate and documented in the payload-version doc comment, though a future
  envelope-layout change must remember to bump the outer constant separately.

## 6. Other observations

- The two new modules (`vdp.rs`, `interrupts.rs`) are untracked in the working tree.
  This is expected for an uncommitted stage review; the final commit must include
  both.
- `InterruptScheduler::advance` documents that its cost is proportional to completed
  scanlines, and a caller-supplied `quanta` near `u64::MAX` would therefore iterate
  for an impractically long time instead of returning a typed error. This is not
  reachable from the backend's one-tick-per-step path; if the public API is used
  directly, a bound/guard would be worth adding later.
- The VDP snapshot format (`CFVDP001`, version 1, 65 574 bytes on the wire; the
  capacity calculation includes the header plus VRAM) and the scheduler snapshot
  format (`CFINTSCH`, version 1, 56 bytes plus pending tick fields) are internally
  consistent; both restore paths validate before mutating and both restore
  methods are exercised by malformed-input tests.
- The CLI demo's numeric assertions match the stored program/CPU behaviour I traced
  instruction by instruction (VDP register 15 = 2, VRAM word `0x1234` read back,
  status `0x0100`, HBlank pending at tick 64, VBlank at tick 896 with quantum 4).

## 7. Verdict

**Needs revision.** The integration is architecturally sound, the requested gate and
demo pass, snapshot/fork/determinism claims are covered by executable tests, the
device state is genuinely included in state/config hashes, and the implemented
capabilities are reported honestly. It does not pass review because:

1. the claimed `LEA`/`PEA (abs).L` decoder also accepts (and mis-executes) the
   absolute-word/PC-relative encodings, and its unit tests use the wrong encoding
   (F1, medium); and
2. authoritative onboarding/architecture documents still state that VDP and
   interrupts are unimplemented and that the hash covers only synthetic state, which
   contradicts the slice (F2, medium).

F3–F5 are low-severity documentation/consistency cleanups that should be fixed in
the same pass. Once F1 and F2 are corrected and the gate is re-run, the review can
be re-opened for a pass verdict.
