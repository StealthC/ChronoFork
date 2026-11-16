# M3 Real-Hardware Input TAS — Executable Acceptance Plan

- **Status:** **Executed; every row M3-A45–M3-A53 is `executed - passed`** (independent
  t5 verification PASS, 2026-11-16). The hardware-TAS increment and the M3 completion audit
  passed for the documented deterministic minimal-slice scope; M3 is complete per the t6
  promotion, and the t7 promotion audit returned PASS on 2026-11-16 (ignored
  `.research/m3-hardware/final-verification/SUMMARY.txt`).
- **Date:** design 2026-11-15; execution record 2026-11-16
- **Companion contract:** [ADR-0009: real-hardware input TAS](../adr/0009-real-hardware-input-tas.md)
  — **Accepted** (owner-approved 2026-11-15T23:38:06-03:00) and **implemented and
  independently verified** (t5 PASS, 2026-11-16).
- **Extends:** the accepted M3 contracts [ADR-0004](../adr/0004-tas-movie-format.md),
  [ADR-0005](../adr/0005-partial-replay-and-branch-comparison.md),
  [ADR-0006](../adr/0006-movie-format-v2-anchors-and-multi-device-input.md),
  [ADR-0007](../adr/0007-in-process-rewind-and-interactive-stepping.md) and
  [ADR-0008](../adr/0008-rerecord-and-branch-editing.md). Their executed rows
  (M3-A1–A12, A13–A17, A18–A27, A28–A34, A35–A44) are **regression preconditions**, never
  acceptance for M3-A45–A53.
- **Related:** [Mega Drive 68000 slice](../architecture/megadrive-slice.md), [execution
  model](../architecture/execution-model.md), [session model](../architecture/session-model.md),
  [functional requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [roadmap](../roadmap/roadmap.md), [M3 first-increment acceptance
  plan](m3-tas-replay-acceptance.md), [M3 increment-2 acceptance
  plan](m3-partial-replay-acceptance.md), [M3 format-v2 acceptance
  plan](m3-format-v2-acceptance.md), [M3 rewind acceptance
  plan](m3-rewind-acceptance.md), [M3 rerecord acceptance
  plan](m3-rerecord-acceptance.md), [research: TAS and
  debugging](../research/tas-and-debugging.md), [oracle differential
  evidence](../research/oracle-differential-evidence.md), [testing strategy](testing-strategy.md),
  [quality gates](quality-gates.md), [anti-slop policy](anti-slop-policy.md).

> **Execution record (2026-11-16).** The independent t5 verifier executed every row on
> HEAD `85af5238fabf204bbb885df3823c604212edd948` (branch `agent/m3-complete`) plus the
> increment working-tree paths, with `CARGO_HOME=/tmp/t7-cargo`, and returned **PASS**
> (2026-11-16T03:04:06Z–03:27Z). Every row is `executed - passed`; raw evidence is under
> the ignored `.research/m3-hardware/verification/` path (1057 artifacts,
> `MANIFEST.sha256` with `sha256sum -c` exit 0) and the dated PASS block is in
> [quality gates](quality-gates.md). The pre-change baseline under
> `.research/m3-hardware/baseline/` was verified before and after the run. The verifier's
> only tracked edit is the dated quality-gates block; no commit, push, tag or merge was
> made. The t6 promotion records these statuses and the M3 completion; the t7 promotion
> audit returned PASS on 2026-11-16 (ignored
> `.research/m3-hardware/final-verification/SUMMARY.txt`).

## 1. Status rules

1. **`planned - not yet executed`** meant, at design time, a criterion with a planned test
   target, command and evidence path but no executed result or independent verifier record.
   No row has this status after the t5 execution record; it is retained as the rule for any
   future row.
2. A row becomes **`executed - passed`** only when all of the following are recorded: the
   exact command, exit code, tree state (HEAD plus uncommitted paths), the test-output
   summary, the ignored evidence artifact path and an independent verifier review that
   returns PASS. The authoritative dated gate block belongs in
   [quality gates](quality-gates.md); this document records row-level execution references
   and a dated execution snapshot if one exists.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error, timeout,
   missing artifact or unverified claim; a failed row blocks this increment. A known
   failure is never converted into a skip, an `#[ignore]`, a relaxed assertion or a
   narrower claim after the fact.
4. A test may be added before the implementation as a red test only if it fails for the
   expected typed reason. A test whose target or module does not exist is not acceptance
   evidence; the row remains `planned - not yet executed`.
5. No controller, `--rom`, hardware-TAS or canonical-non-change claim may be described as
   implemented, working, supported or verified on the strength of this plan, ADR-0009, a
   code review, a demo or a single happy-path run. Executed artifacts are the only
   evidence; M3-A1–A44 are evidence for prior increments only.
6. A green M3-A1–A44 run is a regression precondition, never acceptance for A45–A53.
   Existing `tas record|play|verify|compare|studio|edit` on the synthetic backend does not
   implement hardware input.
7. A row marked passed must cite a verifier artifact whose SHA-256 matches the recorded
   manifest. Late or reconstructed evidence does not reopen a failed row.
8. **ADR-0009 is Accepted and implemented.** Acceptance of the ADR was a decision; the
   executed rows below are the implementation evidence. The owner approval is recorded in
   ADR-0009, and no implementation preceded it.
9. Acceptance is limited to the minimal Mega Drive slice on the Linux acceptance platform
   and the independently authored synthetic controller ROM defined in §2. No commercial
   ROM, real controller hardware, TH/6-button/Start multiplexing, controller timing,
   rendering, audio, Z80, DMA or cycle accuracy is in scope.
10. M3 completion required the separate audit in §9 and ADR-0009 §10. That audit was
    executed by t5 and passed; this plan's execution record and the t6 promotion are the
    durable record, and the t7 promotion audit returned PASS on 2026-11-16.

## 2. Scope, definitions and evidence

- **Accepted backend:** `chronofork.megadrive.m68000`, the minimal 68000 + ROM/RAM + VDP
  port + instruction-quantum interrupt-scheduler slice. The synthetic
  `chronofork-test-backend` remains the acceptance backend for the existing M3-A1–A44 rows
  and is unchanged by this plan.
- **Controller contract:** per-step, stateless reads of the two Mega Drive data registers
  `0x00A10003` (port 1, from `InputFrame::ports()[0]`) and `0x00A10005` (port 2, from
  `ports()[1]`), byte reads only. Bit layout is the TH-high phase of a 3-button pad:
  bits 0–5 = Up/Down/Left/Right/B/C active-low, bit 6 = TH high (1), bit 7 = 0; value
  `0x40 | ((!mask & 0x3F) & 0x3F)`, neutral `0x7F`. A (frame bit 6) and Start (frame
  bit 7) are inert. See [ADR-0009 §2](../adr/0009-real-hardware-input-tas.md#2-controller-model-and-exact-register-contract).
- **Canonical state:** the Mega Drive payload (`CFMD68K2`, payload version 2), state body
  (D0–D7, A0–A7, PC, SR, halted, tick, 64 KiB RAM, VDP snapshot, scheduler snapshot),
  `CFMDSTATE2` state hash and `CFMDCONF2` configuration hash. They must not change.
- **Transient wiring:** a private step-scoped bus field set from the step's `InputFrame`
  and cleared on every step exit path; never serialized, hashed, inspected or copied into
  a snapshot. See [ADR-0009 §4](../adr/0009-real-hardware-input-tas.md#4-backend-wiring-transient-step-scoped-non-serialized).
- **Synthetic controller ROM:** a deterministic, independently authored 68000 program
  built in code (`controller_rom()` beside `demo_rom()`/`interrupt_rom()`), which
  reads both data registers in a loop, accumulates the observed bytes into separate data
  registers, stores the accumulators at fixed work-RAM addresses (`0x00FF0000`,
  `0x00FF0001`) and self-loops. It performs no controller/VDP/SR writes and uses only
  implemented instruction forms. It is not a hardware read routine: it reads only the
  TH-high phase. See [ADR-0009 §6](../adr/0009-real-hardware-input-tas.md#6-hardware-tas-acceptance-design).
- **Baseline:** the pre-change baseline is the ignored `.research/m3-hardware/baseline/`
  path captured on HEAD `85af5238fabf204bbb885df3823c604212edd948` (its capture log records
  `overall_exit=0` for gate, demos, fixtures, oracle and state probes). It includes
  `gate/` (including `test-target-counts.tsv` and per-command stdout/stderr/exit),
  `demos/`, `fixtures/`, `oracle/` and `state-probes/`. The execution compared against it;
  a missing or edited baseline stops the run.
- **Evidence root:** `.research/m3-hardware/verification/` with `SUMMARY.txt`,
  `rows/A45-*.txt`–`A53-*.txt`, `targets/`, `cli/`, `hardware/`, `oracle/`, `demos/`,
  `fixtures/`, `probes/`, `gate/`, `audit/`, `tree/` and `MANIFEST.sha256` (1057 artifacts;
  `sha256sum -c` exit 0).

## 3. Increment exit criteria map

| Verified capability | Acceptance criteria |
|---|---|
| Controller read semantics, exact address map/bit layout, step scope, canonical-state non-change | M3-A45 |
| Hardware `record`/`play`/`verify` end to end, input-dependent state, divergence/compare controls | M3-A46 |
| Hardware movie snapshot/restore/fork/rewind/`studio` and non-mutation | M3-A47 |
| Two-controller input mapping and inert schema-2 fields | M3-A48 |
| CLI `--rom` contract, bounded read, default-path equality and typed errors | M3-A49 |
| Determinism, debug/release equality, bounds, typed errors and structural audit | M3-A50 |
| M1/M2/M3 non-regression, demos, fixtures and full gate | M3-A51 |
| Oracle differential harness gate and unchanged M1 evidence | M3-A52 |
| Scope honesty, deferred list and no M3-complete claim | M3-A53 |

Every mapped row is `executed - passed` (t5 PASS, 2026-11-16) with commands and artifacts
in §4–§5 and §10.

## 4. Executed test targets and commands

These are the executed lowest-useful-layer targets (t5, 2026-11-16). Every Cargo command
used `CARGO_HOME=/tmp/t7-cargo` and is recorded with its exit code, stdout, stderr and
timing in the evidence paths below.

| Target | Command | Criteria |
|---|---|---|
| Bus/read unit tests (additions in `crates/chronofork-megadrive/src/bus.rs`) | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --lib bus::` | M3-A45, A48, A50 |
| Backend wiring/snapshot unit tests (additions in `crates/chronofork-megadrive/src/backend.rs`) | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --lib backend::` | M3-A45, A47 |
| New Mega Drive controller integration target `crates/chronofork-megadrive/tests/controller.rs` | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --test controller -- --nocapture` | M3-A45, A46, A47, A48 |
| New CLI process target `crates/chronofork-cli/tests/tas_hardware.rs` | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_hardware -- --nocapture` | M3-A46, A47, A48, A49, A50 |
| Release-mode controller and CLI targets | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --release --test controller`; `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --release --test tas_hardware` | M3-A50 |
| Existing Mega Drive and CLI targets (regression) | `cargo test -p chronofork-megadrive`; `cargo test -p chronofork-cli`; `cargo test -p chronofork-sessions` | M3-A51 |
| Full gate | `./scripts/check.sh`; `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; `git diff --check` | M3-A51 |
| Demos | `cargo run -p chronofork-cli -- demo` and `cargo run -p chronofork-cli -- megadrive demo`, three cargo runs and three direct-binary runs each, compared with the baseline by `cmp` and SHA-256 | M3-A51 |
| Differential serialized-state probe | compile one probe against a clean `git archive 85af523` export and against the post-change working tree, run it with a neutral (non-controller) ROM and compare payload bytes, payload length, state hash and configuration hash with `cmp` | M3-A45, A51 |
| Oracle differential harness | `python3 scripts/oracle-diff.py --skip-build --chronofork-bin target/debug/chronofork --out-dir .research/m3-hardware/verification/oracle/run` | M3-A52 |
| Fixture/transcript comparison | rerun the baseline `fixtures/` scenarios (v1/v2 record/play/verify/range/compare/edit/studio, durable session probe) and `cmp` movie bytes, movie SHA-256 and stdout transcripts with the baseline | M3-A49, A51 |
| State-probe comparison | run `megadrive trace` twice per oracle ROM with the baseline flag set and `cmp` trace/state files with `.research/m3-hardware/baseline/state-probes/` | M3-A51, A52 |
| Structural and suppression audits | `rg` scans for readers/parsers/predicates, `git diff -U0` for new `#[allow]`/`#[ignore]`/`unsafe`, `cargo tree -d`, `git diff -- Cargo.toml Cargo.lock`, `wc -l` AS-14 check | M3-A50 |
| Scope/help audit | read `tas --help`/`tas <cmd> --help`, the slice/execution docs, ADR-0009 and this plan; compare every claim with executed rows | M3-A53 |

All listed targets exist and were executed. Test counts and timings are recorded only in
the evidence artifacts, never as durable claims in this plan
([anti-slop policy](anti-slop-policy.md) AS-13).

## 5. Acceptance criteria

Every row below is `executed - passed` (independent t5 verification, 2026-11-16). The
pass conditions are retained as the verified contract; each row records the executed
commands, observed result and evidence path. No row is failed.

### M3-A45 — Controller read semantics, address map and canonical-state non-change

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated on the minimal Mega
Drive slice:

1. **Address map, reads.** A byte read of `0x00A10003` returns the port-1 byte derived from
   `ports()[0]`; a byte read of `0x00A10005` returns the port-2 byte derived from
   `ports()[1]`. Every other address in `0x00A10000..=0x00A1001F` — including the version
   register `0x00A10001`, port 3 `0x00A10007`, control registers `0x00A10009`/`0x00A1000B`
   and TX/RX — is a typed unmapped read; a word/long access that touches the window (for
   example `0x00A10002`) is a typed error, not a partial success.
2. **Bit layout and subset.** For masks covering each of Up, Down, Left, Right, B and C,
   the byte equals `0x40 | ((!mask & 0x3F) & 0x3F)`; a neutral mask reads `0x7F`; setting
   only frame bit 6 (A) or bit 7 (Start) does not change the byte. High mask bits are
   ignored. Both ports use the same layout independently.
3. **Writes fail closed.** A byte write to `0x00A10003`/`0x00A10005` and a write to any other
   controller-window address is a typed `UnmappedWrite` (mapped to the existing backend
   execution error), never a silent no-op and never a partial state change.
4. **Step scope and clearing.** During an executed instruction, a controller read observes
   the current step's frame. After `step` returns — on success and on every error path — a
   direct bus read observes the neutral idle byte, never the previous step's input. A
   debugger `read_memory` of the controller window is a typed error (the side-effect-free
   path does not map step-scoped input) and does not change state.
5. **Input is not canonical.** A program that never reads the controller produces the same
   trace, payload bytes, payload length, state hash and configuration hash for every input
   frame it is given, including non-neutral ports. The differential serialized-state probe
   over a neutral non-controller ROM is byte-identical between the pre-change tree and the
   post-change tree (payload, length, state hash, config hash).
6. **Canonical format unchanged.** The payload magic stays `CFMD68K2`, the payload version
   stays 2, the state body field list/order and length are unchanged, and `CFMDSTATE2` /
   `CFMDCONF2` cover exactly the same bytes as before. A pre-change snapshot still restores;
   a rejected restore leaves the machine untouched, exactly as today.
7. **Snapshot does not carry input.** The canonical state-body field list is audited to
   contain no controller byte, port mask or input field; two runs of a non-controller
   program with different frames produce byte-identical payloads (the #5 probe). Restoring
   a post-step checkpoint and stepping with the next input reproduces the uninterrupted
   run's tick and state hash; restoring the same checkpoint and stepping neutral versus
   pressed produces different hashes, proving the next step's input is used rather than a
   stale latch.
8. **No new inspection/capability surface.** `inspect()` registers and
   `backend describe` output are unchanged; no controller pseudo-register or capability bit
   is added.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Bus unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --lib bus:: -- --nocapture` | `targets/megadrive-lib-bus.stdout.txt` |
| Backend/snapshot unit tests | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --lib backend:: -- --nocapture` | `targets/megadrive-lib-backend.stdout.txt` |
| Controller integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --test controller -- --nocapture` | `hardware/controller/` |
| Differential state probe | probe against `git archive 85af523` and the working tree; `cmp` payload/state bytes and lengths | `probes/` |

Executed result (t5): `cargo test -p chronofork-megadrive --lib bus::` (13 passed), `--lib backend::` (38) and `--test controller` (6) all exited 0. The differential probe against `git archive 85af523` produced byte-identical payload/envelope/meta (payload SHA-256 `db6e61db…`, length 131255, state hash `87c57711…`, config hash `a8b7ffb0…`), 23/23 per-step state hashes were identical for neutral vs extreme frames, and a pre-change snapshot restored on the post-change tree (and vice versa). Evidence: `rows/A45-controller-reads-and-canonical-state.txt`, `targets/`, `probes/`, `hardware/controller/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A45-controller-reads-and-canonical-state.txt`).

### M3-A46 — Hardware record, play and verify end to end

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. **Record on the Mega Drive backend.** `tas record --rom <controller-rom> --movie <file>
   --steps <N> --input <mask>` exits 0, prints the existing `record=movie` block with the
   existing fields, writes a canonical v1 movie (`movie_version=1`, `input_schema=1`) and
   leaves the ROM file's bytes and SHA-256 unchanged. A second identical invocation in a
   fresh process (same ROM, steps, mask, fresh manager) produces byte-identical movie bytes
   and the same `movie_sha256`.
2. **Verify and play in a new process.** `tas verify --rom <controller-rom> --movie <file>`
   and `tas play --rom <controller-rom> --movie <file>` each import the embedded Mega Drive
   checkpoint, apply every recorded frame, recompute every tick and state hash, and print
   `verified=1` / the recorded end tick and final state hash. A divergence uses the
   existing typed first-divergence report, not a partial pass.
3. **Input changes canonical state.** Two recordings of the same ROM/steps with different
   port-1 masks (for example neutral versus Up+B) have different per-step and final state
   hashes and different `movie_sha256`; `tas compare --movie <a> --against <b>` (with no
   `--rom`) reports the first divergent index/tick/hash. The first divergence occurs no
   later than the first recorded step whose executed instruction reads the controller; a
   program that never reads it shows identical recorded expectations for different masks.
4. **Observed values are exact.** At the session layer, after a known number of steps with
   known masks, the work-RAM accumulator bytes at `0x00FF0000`/`0x00FF0001` equal the
   defined active-low accumulation; the values are part of the canonical state and are not
   derived from a host time or a hidden input copy.
5. **Wrong backend/configuration fails typed.** `tas verify --rom <controller-rom>` on a
   synthetic movie and `tas verify` (no `--rom`) on the hardware movie exit 1 with a typed
   `IncompatibleSnapshot` diagnostic on stderr and empty stdout; a different ROM of valid
   size (different `CFMDCONF2`) also fails import typed. No fallback, retry or silent
   re-import occurs.
6. **v2 multi-port hardware movie.** A `--ports P1,P2,P3` record on the Mega Drive backend
   produces a schema-2 v2 movie, verifies and plays; the port-1/port-2 values change state,
   while `ports[2..3]`, axes and keys remain inert (see M3-A48). v1 records remain v1.
7. **Source non-mutation and atomic publish.** Recording and replay never write the ROM,
   the source movie or a storage root; a failed record does not replace an existing movie
   file (the existing sibling-temp+rename contract).

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| CLI process target | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_hardware -- --nocapture` | `cli/hardware/` |
| Scripted CLI transcript | `tas record`/`play`/`verify`/`compare` with the generated `controller_rom()` file, two masks, fresh processes | `cli/hardware/transcripts/` |
| Session-level observation | controller integration test inspecting RAM and state hashes | `hardware/controller/observed-values.txt` |

Executed result (t5): the CLI hardware target passed 13 tests in debug and release; hardware record/play/verify transcripts, deterministic re-record and observed RAM values passed. Neutral/port-1 final hash `6d1677d5…` differs from Up+B `2d97debc…`; compare reports divergence index 4; the identical re-record movie SHA-256 is `42fcda60…`; the ROM SHA-256 `b5596069…` is unchanged and the ROM is not embedded. Evidence: `rows/A46-hardware-record-play-verify.txt`, `cli/hardware/`, `probes/controller/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A46-hardware-record-play-verify.txt`).

### M3-A47 — Snapshot, restore, fork, rewind and studio on a hardware movie

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. **Snapshot/restore equality.** With a manager over `MegadriveFactory` and the controller
   ROM, run K steps, snapshot, run M more steps, restore the checkpoint and rerun the same
   M frames: every tick and state hash equals the uninterrupted run. The restored machine's
   first step uses the supplied next frame, not a serialized one.
2. **Fork isolation and divergence.** Fork from one checkpoint; step the source and each
   fork with the next frames (for example fork A port 1 neutral, fork B Up+B). The forks'
   state hashes differ at the first controller-reading step, the source's tick/hash are
   unchanged, and no fork mutates another.
3. **Rewind exactness.** A `tas studio --rom <controller-rom> --movie <hardware-movie>`
   run using `next`/`back`/`goto`/`state`/`quit` lands on the recorded tick and state hash
   at each target, equal to a fresh `tas verify`/ranged replay at that target, including a
   target before the earliest cached state and after an eviction. The cache bounds and
   metrics follow the existing ADR-0007 contract.
4. **Studio non-mutation.** The studio run writes no file, storage root or cache; the movie
   SHA-256 and directory contents are unchanged; a `tas verify --rom` after the studio run
   still passes.
5. **Wrong factory is typed.** `tas studio` without `--rom` on a hardware movie, and with
   `--rom` on a synthetic movie, exit 1 with the typed incompatible-checkpoint diagnostic
   and no block on stdout for the failed open.
6. **No canonical change from these paths.** Snapshot bytes, payload version, state hashes
   and movie bytes are unaffected by studio, fork or rewind beyond the input-dependent
   machine state they already execute.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Controller integration (snapshot/fork) | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --test controller -- --nocapture` | `hardware/controller/snapshot-fork.txt` |
| Studio process test | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_hardware -- --nocapture` | `cli/hardware/studio/` |
| Scripted studio transcript | `tas studio --rom <rom> --movie <movie>` with fixed stdin script | `cli/hardware/studio/script.*` |

Executed result (t5): controller 6 and CLI 13 tests passed; snapshot/restore/fork with the next frame, scripted studio (11 blocks, 2 cache evictions, goto-2 equal to ranged verify), no written files and typed wrong-factory failures all passed. Evidence: `rows/A47-snapshot-restore-fork-rewind-studio.txt`, `hardware/controller/snapshot-fork.txt`, `cli/hardware/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A47-snapshot-restore-fork-rewind-studio.txt`).

### M3-A48 — Two-controller input and inert schema-2 fields

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. **Port mapping.** Controller 1 (`0x00A10003`) is driven only by `ports()[0]`; controller
   2 (`0x00A10005`) only by `ports()[1]`. A mask on port 1 changes only the port-1
   accumulator byte; a mask on port 2 changes only the port-2 accumulator byte; both
   together change both.
2. **Two-controller recording.** `tas record --rom <rom> --movie <file> --steps <N>
   --input <p1> --ports <p2>,0,0` records and verifies a v2 movie whose replay produces the
   same port-1/port-2-dependent state hashes; two different `p2` values with the same `p1`
   produce different hashes and a compare divergence.
3. **Inert fields.** Non-neutral `ports()[2]`, `ports()[3]`, any `axes()` value and any
   `keys()` bitmap produce no state-hash, trace or RAM change versus neutral values; they
   are recorded in v2 movies and ignored by the Mega Drive backend, never reinterpreted as
   buttons or hardware.
4. **A/Start limitation.** Setting only frame bit 6 (A) or bit 7 (Start) on either port
   changes no observed controller byte and no state hash; the limitation is stated in help
   and docs.
5. **Port 3 is absent.** A read or write of `0x00A10007` is a typed unmapped access; no
   third controller is modelled.
6. **Schema preservation.** v1 movies reconstruct neutral extra fields; v2 multi-device
   records round-trip through the existing codec; no movie byte, schema version or limit
   changes.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Bus port mapping | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --lib bus:: -- --nocapture` | `targets/megadrive-lib-bus.stdout.txt` |
| Controller integration | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-megadrive --test controller -- --nocapture` | `hardware/controller/two-ports.txt` |
| CLI two-controller record | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_hardware -- --nocapture` | `cli/hardware/two-controller/` |

Executed result (t5): bus 13, controller 6 and CLI 13 tests passed; probe values `[7e,5f,7e,5f]` vs swapped `[5f,7e,5f,7e]` prove independent ports; v2 anchors=3 and a port-2 change diverges at index 8; A/Start, ports[2..3], axes and keys are inert; port 3 is typed unmapped; v1/v2 schemas are preserved. Evidence: `rows/A48-two-controller-and-inert-fields.txt`, `hardware/controller/two-ports.txt`, `cli/hardware/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A48-two-controller-and-inert-fields.txt`).

### M3-A49 — CLI `--rom` contract and typed errors

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. **Accepted commands.** `tas record`, `play`, `verify`, `studio` and `edit` accept
   `--rom <FILE>`; `tas compare` rejects it as an unknown option and remains
   backend-agnostic. Missing values, duplicate values, empty values and `--rom` on unrelated
   commands are typed CLI failures with empty stdout and exit 1.
2. **Default path byte-identical.** With no `--rom`, every existing synthetic command
   produces byte-identical movie files, movie SHA-256 values and stdout transcripts to the
   pre-change baseline (v1/v2 record/play/verify/range/compare/edit/studio fixtures), and
   the default `tas --help`/subcommand help changes only by additive documentation of the
   new flag.
3. **Bounded ROM read before allocation.** A missing path, a directory, an unreadable file,
   a file shorter than 8 bytes and a file larger than the 4 MiB cartridge window each fail
   typed before constructing an oversized allocation or a factory; the file is opened
   read-only, read with a bound and never written, renamed or deleted. No panic occurs on
   arbitrary file sizes.
4. **Wrong backend/configuration.** A synthetic movie with `--rom`, a hardware movie
   without `--rom`, a hardware movie with a different same-size ROM and a hardware movie
   with a different scheduler configuration all fail with the typed
   `SessionError::IncompatibleSnapshot` diagnostic and exit 1.
5. **Storage combination rejected.** `tas record --root ... --rom ...` is a typed CLI
   failure before any storage read or session creation; the existing CLI cannot publish a
   Mega Drive session to durable M2 storage, so the combination would only produce a
   confusing import incompatibility. Durable storage stays a synthetic-surface command
   path in this increment.
6. **No ROM mutation or embedding.** The ROM file's SHA-256 and metadata are unchanged
   across record/play/verify/studio/edit; no ROM bytes appear in a movie, stdout, stderr or
   storage root; the movie carries only the configuration hash (so the same ROM is needed
   to replay).
7. **Output and streams unchanged.** Successful commands keep their existing `key=value`
   blocks, field order and exit codes; no host path, ROM bytes or timestamp appears in
   valid stdout; diagnostics stay on stderr as `chronofork: <error>`; a failed record does
   not replace an existing movie.
8. **Edit and studio integration.** `tas edit --rom <rom> --movie <hardware-movie> --out
   <branch> --set-input <i>=<mask>` produces a branch that verifies with the same ROM,
   leaves the source movie unchanged and records the correct parent; `tas studio --rom`
   opens the same movie as M3-A47.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| CLI process target | `CARGO_HOME=/tmp/t7-cargo cargo test -p chronofork-cli --test tas_hardware -- --nocapture` | `cli/rom-contract/` |
| Scripted flag/error matrix | scripted CLI runs for accepted/unknown/duplicate/missing flags, missing/dir/short/large ROM, wrong ROM, no `--rom`, `--root --rom` | `cli/rom-contract/transcripts/` |
| Default-path fixture comparison | rerun baseline `fixtures/` scenarios and `cmp` movie bytes, SHA-256 and transcripts | `fixtures/default-path/` |

Executed result (t5): 13 CLI tests, a 216/216-check matrix and 18 typed-error transcripts passed; the default path retained 59/59 transcripts and 19/19 movies byte-identical to baseline; the ROM was read-only (chmod 444) and never embedded; help changes were additive plus the honest deferred list. Evidence: `rows/A49-rom-cli-contract.txt`, `cli/hardware/`, `fixtures/comparison/`, `audit/help-diff.txt`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A49-rom-cli-contract.txt`).

### M3-A50 — Determinism, bounds, typed errors and structural audit

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. **Determinism.** Two recordings with the same ROM/steps/masks in separate fresh
   processes are byte-identical; debug and release builds produce identical movie bytes,
   state hashes and stdout blocks; repeated verify/studio runs produce identical output;
   no host time, environment variable, process id, path or entropy enters canonical bytes
   or valid stdout.
2. **Bounds.** The ROM read is bounded before allocation; controller reads are constant
   work per access; no new allocation is sized from an unvalidated declared length; movie,
   snapshot, checkpoint, anchor and step limits are unchanged. Overflow is a typed error,
   never a wrap or panic.
3. **Typed errors.** Every failure path in the §8 table of
   [ADR-0009](../adr/0009-real-hardware-input-tas.md#8-limits-and-typed-errors) is a typed
   error preserving its source; no stringly flattened `SessionError`/`BackendError` and no
   `unwrap`/`expect` in production paths.
4. **No structural drift.** Exactly one controller read implementation, no second
   snapshot parser, no second compatibility predicate and no second manager/import path;
   the new fixture builder has test callers; no speculative public API.
5. **No suppression or dependency.** `git diff` adds no `#[allow]`, `#[ignore]`, `unsafe`,
   deleted test or relaxed lint; `Cargo.toml`/`Cargo.lock` are unchanged; `cargo tree -d`
   is empty; no new crate or dependency.
6. **Module responsibility.** New/changed modules are split by responsibility and stay
   below or documented against the AS-14 800-non-test-line review trigger; measured counts
   belong to evidence, not this plan (AS-13).
7. **Structural checks run.** `rg` definition counts for the reader/writer, parser,
   predicate and controller read; `git diff -U0` suppression scan; `wc -l` AS-14 check;
   all recorded with commands and outputs.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Determinism (debug/release) | controller + CLI targets in debug and `--release`; scripted repeat runs | `hardware/determinism/`, `cli/determinism/` |
| Structural audit | separate `rg` scans for `ControllerInput`, `fn read8`, `fn peek8`, `validate_envelope` and `from_bytes`; `git diff -U0`; `cargo tree -d`; `wc -l` | `audit/` |
| Suppression/dependency audit | `git diff -- Cargo.toml Cargo.lock`; `git diff -U0` plus an added-line scan for `#[allow]`, `#[ignore]` and `unsafe` | `audit/` |

Exact executed audit commands (run from the repository root, output captured under
`audit/`):

```sh
rg -n "ControllerInput" crates/chronofork-megadrive
rg -n "fn read8" crates/chronofork-megadrive
rg -n "fn peek8" crates/chronofork-megadrive
rg -n "validate_envelope" crates
rg -n "from_bytes" crates/chronofork-sessions
git diff -- Cargo.toml Cargo.lock
git diff -U0 | rg "^\+.*(allow\(|#\[ignore|unsafe)"
cargo tree -d
wc -l crates/*/src/*.rs
```

Executed result (t5): debug and release movie bytes were identical (`3e11e762…`); bounds and typed errors were observed; the structural audit found one controller/manager path; no `#[allow]`/`#[ignore]`/`unsafe`/deleted test/dependency was added; AS-14 counts are recorded as evidence. Evidence: `rows/A50-determinism-bounds-structure.txt`, `audit/`, `targets/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A50-determinism-bounds-structure.txt`).

### M3-A51 — M1/M2/M3 non-regression, demos and fixtures

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated against the captured
baseline:

1. **Full gate.** `./scripts/check.sh` exits 0 with `cargo fmt --all -- --check`,
   `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
   and `git diff --check` all clean.
2. **Target matrix.** Every test target is at or above the baseline count in
   `.research/m3-hardware/baseline/gate/test-target-counts.tsv`; every delta is explained
   by the new targets, and no existing assertion is removed or weakened.
3. **Demos byte-identical.** `demo` and `megadrive demo`, three cargo runs and three
   direct-binary runs each, are byte-identical across runs and `cmp`-identical to the
   baseline, with equal SHA-256.
4. **Mega Drive non-regression.** Existing `chronofork-megadrive` unit/integration tests
   pass; the three oracle ROM trace/state probes are byte-identical to the baseline
   `state-probes/`; the oracle ROMs are audited not to access the controller window; the
   differential serialized-state probe for a neutral non-controller program is
   byte-identical (payload, length, `CFMDSTATE2`, `CFMDCONF2`).
5. **M3 movie/CLI non-regression.** The baseline v1/v2 record/play/verify/range/compare/
   edit/studio fixtures produce byte-identical movies, `movie_sha256` values and stdout
   transcripts; the existing M3 acceptance targets still pass.
6. **M2 non-regression.** The durable session fixtures (save/load/list/delete on two
   independent roots) are byte-identical to the baseline and the M2 targets still pass.
7. **No hidden format change.** Payload magic/version, state body, hashes, snapshot sizes,
   movie bytes, schema/format versions and limits are identical to the baseline for the
   same inputs; a deliberate difference is a stop condition, not an explanation.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Full gate | `./scripts/check.sh` and each listed gate command, exit codes recorded | `gate/` |
| Test matrix | `cargo test --workspace`; compare with baseline `test-target-counts.tsv` | `gate/target-matrix.txt` |
| Demos | three runs each, baseline comparison | `demos/` |
| Fixtures and state probes | baseline reruns and `cmp` | `fixtures/`, `probes/` |
| Oracle ROM audit | inspect generated ROM/program sources for `0x00A10000` access | `audit/oracle-roms.txt` |

Executed result (t5): all seven gate commands exited 0; `cargo test --workspace` was 532 passed / 0 failed / 0 ignored over 39 targets (baseline 502/37) and `check.sh` reported the same; demos (986 B `9b4dd564…`, 1582 B `912a5bb6…`), 59/59 fixtures, 19/19 movies, the two-root session probe and all three state probes are byte-identical to baseline. Evidence: `rows/A51-m1-m2-m3-non-regression.txt`, `gate/`, `demos/`, `fixtures/`, `state-probes/`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A51-m1-m2-m3-non-regression.txt`).

### M3-A52 — Oracle differential harness gate

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. The pinned Exodus oracle environment (`.research/upstream/exodus-headless` and
   `.research/oracle/build` with the documented binary hashes) is present and matches the
   recorded revisions; if it is absent, the run stops with an explicit blocker and does not
   mark the row passed.
2. `python3 scripts/oracle-diff.py --skip-build --chronofork-bin target/debug/chronofork
   --out-dir .research/m3-hardware/verification/oracle/run` exits 0 with
   `status: pass`: `rom_alu`, `rom_mem_vdp` and `rom_masked` have identical projected
   address streams, equal selected final state, passing negative controls and the
   scheduler/masked-gating control, exactly as the baseline `oracle/run/oracle-diff.json`.
3. Per-ROM projected counts, projection SHA-256 values, final-state values, negative-control
   indices and the masked-ROM quantum/mask/boundary control are identical to the baseline;
   no comparison is weakened, skipped or re-baselined to obtain a pass.
4. The oracle ROMs do not access the controller window, so the controller mapping cannot
   change their traces; the audit is recorded and the harness source is unchanged
   (SHA-256 recorded).
5. A real divergence is reported as a FAIL finding and is never masked by editing the
   core, the comparison or the evidence.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Oracle harness | `python3 scripts/oracle-diff.py --skip-build --chronofork-bin target/debug/chronofork --out-dir .research/m3-hardware/verification/oracle/run` | `oracle/run/`, `oracle/stdout.txt`, `oracle/exit.txt` |
| Baseline comparison | `cmp`/JSON field comparison against `.research/m3-hardware/baseline/oracle/run/` | `oracle/comparison.txt` |
| ROM audit | inspect the generated ROM sources/bytes for controller addresses | `audit/oracle-roms.txt` |

Executed result (t5): `oracle-diff.py --skip-build` exited 0 with `status=pass`; 3/3 traces, 3/3 final states, 3/3 negative controls, 3/3 scheduler controls and masked gating 1/1 passed; the JSON comparison against baseline produced 0 DIFF lines; the ROMs contain no controller-window access. Evidence: `rows/A52-oracle-differential.txt`, `oracle/`, `state-probes/`, `audit/oracle-roms.txt`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A52-oracle-differential.txt`).

### M3-A53 — Scope honesty and deferred list

**Executed and passed (t5, 2026-11-16).** The verified contract was: all of the following are demonstrated:

1. `docs/architecture/megadrive-slice.md`, `docs/architecture/execution-model.md`,
   `docs/requirements/functional.md`, `docs/roadmap/milestones.md`,
   `docs/roadmap/roadmap.md`, `docs/README.md`, the ADR index and the CLI help describe
   exactly the executed controller subset and `--rom` surface, and claim no unimplemented
   behavior.
2. The deferred list (TH/6-button/Start multiplexing, A/Start, controller timing,
   analog/keyboard hardware, port 3, real commercial ROMs, cycle accuracy, import/export
   adapters) is present in ADR-0009 §10 and in help/docs, and no deferred item is described
   as implemented.
3. ADR-0009 status matches its evidence: **Accepted** (owner-approved 2026-11-15),
   **implemented and independently verified** (t5 PASS 2026-11-16), with M3 completion
   recorded by the t6 promotion; the t7 promotion audit returned PASS on 2026-11-16.
4. No ROM, BIOS, commercial asset or secret is tracked or embedded; the acceptance ROM is
   independently authored synthetic code generated in code.
5. The evidence manifest binds the tree under test; no commit, push, tag or merge is
   claimed without separate authorization; late or reconstructed evidence is not attached
   to a passed row.
6. If any row in this plan is failed or unexecuted, no document says the increment is
   complete or that M3 is complete.

**Executed targets and commands:**

| Target | Command | Evidence |
|---|---|---|
| Docs/help audit | read the touched docs and `tas --help`/subcommand help; compare claims with rows | `audit/docs-claims.txt` |
| Asset audit | `git status --ignored`, tracked-file scan for ROM/BIOS assets, `git diff --stat` | `audit/assets.txt` |
| Evidence binding | `sha256sum` manifest and tree-state record | `tree/`, `MANIFEST.sha256` |

Executed result (t5): the docs/help honesty and asset audits passed (no ROM/BIOS/secret, synthetic acceptance ROM, deferred list intact) and the evidence manifest binds the tree under test. The only pending half was the durable-doc promotion, which this t6 task completes. Evidence: `rows/A53-scope-honesty.txt`, `audit/docs-claims.txt`, `audit/assets.txt`, `tree/`, `MANIFEST.sha256`.

**Status:** executed - passed (independent t5 verification, 2026-11-16; row record `rows/A53-scope-honesty.txt`).

## 6. CLI surface (implemented and verified)

The implemented additive surface is described in
[ADR-0009 §5](../adr/0009-real-hardware-input-tas.md#5-cli---rom-backend-selection). At
design time no flag exists.

```text
chronofork tas record  ... [--rom <FILE>]
chronofork tas play    ... [--rom <FILE>]
chronofork tas verify  ... [--rom <FILE>]
chronofork tas studio  ... [--rom <FILE>]
chronofork tas edit    ... [--rom <FILE>]
chronofork tas compare ...                  # no --rom
```

- Removing `--rom` preserves every existing output block, field order, exit code, movie
  byte and help line except the additive new-flag documentation.
- With `--rom`, no new stdout field was added: the existing blocks already name the movie,
  ticks, steps, state hash and movie SHA-256. The ROM path and ROM bytes must never appear
  in valid stdout.
- `--rom` failures are stderr-only `chronofork: <error>` diagnostics with exit 1 and empty
  stdout; divergence keeps its existing stdout block behavior.
- `record --rom --root` is a typed CLI error; `compare --rom` is an unknown-option error.

## 7. Baseline preconditions

1. The pre-change baseline under `.research/m3-hardware/baseline/` must exist unmodified
   for the tree under test; the recorded capture log reports `overall_exit=0` for gate,
   demos, fixtures, oracle and state probes. A missing, edited or mismatched baseline is a
   stop condition.
2. The tree under test must be the same checkout/branch and must record HEAD plus every
   uncommitted path. A docs-only change does not bind implementation bytes; a later
   implementation edit invalidates any earlier PASS until re-verified.
3. The oracle harness environment and the three ROM artifacts must match the recorded
   pinned revisions/hashes; absence is a typed blocker for M3-A52 and is never a pass.
4. The baseline is evidence for comparison only; it is never acceptance for A45–A53 and is
   never edited to manufacture a match.

## 8. Traceability

| Plan row | Requirements | ADR sections |
|---|---|---|
| M3-A45 controller reads and canonical non-change | FR-MD-004, FR-MD-005, FR-EXEC-002, FR-EXEC-003, NFR-DET-003, NFR-ISO-005, NFR-QUAL-007 | ADR-0009 §2–§4, §8 |
| M3-A46 hardware record/play/verify | FR-MD-010, FR-TAS-001, FR-EXEC-004, FR-EXEC-005, NFR-DET-001, NFR-DET-003 | ADR-0009 §5–§6 |
| M3-A47 snapshot/restore/fork/rewind/studio | FR-SNAPSHOT-003, FR-SNAPSHOT-004, FR-FORK-001, FR-FORK-005, FR-EXEC-007, FR-EXEC-008, NFR-ISO-001, NFR-ISO-002 | ADR-0009 §3–§6; ADR-0007 unchanged |
| M3-A48 two-controller and inert fields | FR-MD-010, FR-TAS-004, NFR-DET-003, NFR-QUAL-007 | ADR-0009 §2, §7 |
| M3-A49 `--rom` contract and errors | FR-API-001, FR-MD-010, FR-TAS-001, NFR-QUAL-004, NFR-SEC-001 | ADR-0009 §5, §8 |
| M3-A50 determinism, bounds, typed errors, structure | NFR-DET-001, NFR-DET-003, NFR-ISO-003, NFR-QUAL-001, NFR-QUAL-004, NFR-QUAL-006, AS-01, AS-02, AS-05, AS-09, AS-10, AS-11, AS-12, AS-14, AS-15 | ADR-0009 §4, §8 |
| M3-A51 M1/M2/M3 non-regression, demos, fixtures | NFR-DET-004, NFR-DUR-001, NFR-QUAL-001, NFR-QUAL-005, M1/M2/M3-A1–A44 | ADR-0009 §3, §6 |
| M3-A52 oracle gate | FR-MD-012, NFR-DET-005, NFR-DOC-004 | ADR-0009 §3, §6; [oracle evidence](../research/oracle-differential-evidence.md) |
| M3-A53 scope honesty and deferred list | NFR-QUAL-005, NFR-DOC-001, NFR-DOC-003 | ADR-0009 §10–§11 |

## 9. Out of scope and stated limitations

**Out of scope for this increment (never to be claimed):**

- TH latch/direction register, TH-low phase, A and Start buttons;
- 6-button pad (X/Y/Z/Mode), detection/presence protocols and the 1.5 ms reset window;
- real controller timing, per-scanline/per-frame input, emulated-time input events and
  controller interrupts;
- analog axes, keyboard hardware, port 3, expansion/peripheral protocols and adapters;
- real commercial ROMs, ROM headers/region/TMSS, cartridge mappers and game compatibility;
- cycle accuracy, bus-cycle/prefetch/VDP timing, rendering, audio, Z80 and DMA;
- import/export adapters for other TAS formats;
- a controller capability bit, new inspection pseudo-registers, durable Mega Drive
  sessions in M2 storage, `record --root --rom`;
- cross-backend canonicalization, cross-platform runtime determinism and any
  hardware-accuracy claim.

**Verified limitations and deferred scope:**

- the slice implements a six-button subset (Up/Down/Left/Right/B/C) per port; A and Start
  are absent;
- a real-hardware read routine that toggles TH cannot run unmodified; controller-window
  writes are typed errors;
- controller input is per-step and not part of checkpoint identity; a snapshot alone does
  not record what was held, and a debugger cannot inspect a live controller;
- a hardware movie requires the exact ROM file to replay; the ROM is not embedded;
- the acceptance claim is the deterministic minimal slice on Linux, not a controller
  protocol, commercial-ROM or timing claim;
- M3 is complete for the documented deterministic minimal-slice scope; this plan does not
  make any deferred capability complete or claimed.

**M3 completion audit (executed 2026-11-16):** ADR-0009 has an explicit owner approval;
all A45–A53 rows are `executed - passed` against a bound tree and manifest; the M1 oracle,
demos, fixtures, M2/M3 evidence and canonical bytes are unchanged; the full gate is green;
and the deferred list above is still honestly unimplemented. The t5 completion audit covered
the five remaining M3 milestone items and passed (`SUMMARY.txt` §9). **M3 is complete for
the documented deterministic minimal-slice scope**, recorded by t6; the t7 promotion audit
returned PASS on 2026-11-16.

## 10. Execution record (2026-11-16)

**Verdict: PASS.** The independent t5 verifier executed M3-A45–M3-A53 and the M3
completion audit on HEAD `85af5238fabf204bbb885df3823c604212edd948` (branch
`agent/m3-complete`) plus the 19 increment working-tree paths, with
`CARGO_HOME=/tmp/t7-cargo`, 2026-11-16T03:04:06Z–03:27Z.

- **Rows:** M3-A45–A53 each `executed - passed`; per-row commands, exit codes, observed
  values and artifacts are in `rows/A45-*.txt`–`A53-*.txt`.
- **Gate:** `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, `./scripts/check.sh` and
  `git diff --check` all exit 0; 532 passed / 0 failed / 0 ignored over 39 targets
  (baseline 502/37); target matrix shows every baseline target at or above its baseline
  count with no failed target; the two new targets are `controller.rs` (6) and
  `tas_hardware.rs` (13).
- **Regression:** both demos (3 cargo + 3 direct-binary runs each), 59/59 fixture
  transcripts, 19/19 movies, the two-root durable-session probe, all three oracle
  state/trace probes, the oracle JSON (0 DIFF lines), the M1 harness (3/3 traces, 3/3
  final states, 3/3 negative controls, 3/3 scheduler controls) and the differential
  serialized-state probe are byte-identical to the pre-change baseline.
- **Canonical non-change:** payload/envelope/meta SHA-256 `db6e61db…`/`87c57711…`/
  `a8b7ffb0…` and length 131255 are identical pre/post for a neutral non-controller ROM;
  23/23 per-step state hashes are identical for neutral vs extreme frames; pre-change
  snapshots restore post-change and vice versa.
- **M3 completion audit:** anchors/seek, rewind/interactive stepping, rerecord/branch
  editing, multi-device input and real-hardware input TAS all have executed evidence
  (`SUMMARY.txt` §9); import/export adapters remain documented-only. M3 is complete for the
  documented deterministic minimal-slice scope, recorded by t6; the t7 promotion audit
  returned PASS on 2026-11-16.
- **Evidence binding:** `.research/m3-hardware/verification/` has 1057 artifacts;
  `MANIFEST.sha256` (1057 entries) verifies with `sha256sum -c` exit 0; the verifier's only
  tracked edit is the dated [quality gates](quality-gates.md) block. No commit, push, tag
  or merge was made.

## 11. Change control

- ADR-0009 is **Accepted** (owner-approved 2026-11-15), implemented and independently
  verified (t5 PASS, 2026-11-16). The owner approval preceded implementation, and the
  implementation followed the staged plan and this acceptance contract.
- A change to the address map, bit layout, mapped access width, write behavior, canonical
  non-change requirement, transient wiring, CLI command set or acceptance rows requires an
  explicit ADR amendment and re-verification; it must not be made by renaming a test,
  mapping an extra register, accepting a silent write or loosening an assertion.
- A failure in any row stops the increment; it is reported as `executed - failed` with the
  exact command, exit code and artifact. It is never converted into a skip, an
  `#[ignore]`, a relaxed assertion or a narrower claim after the fact.
- Any difference in a canonical payload byte, payload version, state body, state hash,
  configuration hash, demo byte, oracle artifact, existing movie byte or existing CLI
  output is a canonical behavior change: stop, re-baseline, version it if canonical, and
  obtain a new owner decision. It is never "explained away".
- The verifier-owned dated [quality gates](quality-gates.md) block is the authoritative
  gate snapshot; this plan records row-level references and does not edit that block.
