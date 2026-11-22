# M2 Durable Sessions — Executable Acceptance Plan

> **Historical record.** This is a dated verification/acceptance or review record. It
> documents what was executed and observed on its own date and is **not current status**.
> Milestone status, integration state and follow-ups live once in
> [Milestones](../roadmap/milestones.md); machine-level Implemented / Not implemented
> lists live in [Mega Drive slice](../architecture/megadrive-slice.md#implemented).
> Where this record disagrees with those pages, they win.

- **Status:** Fase A accepted design; Fase B implementation executed and independently
  verified in two rounds (2026-11-15). Every acceptance row below is
  **executed - passed**; the round-1 CLI lock-scope finding F1 was repaired and
  re-verified closed in round 2.
- **Date:** Fase A design 2026-11-14; execution record 2026-11-15
- **Companion contract:** [ADR-0003: durable session
  storage](../adr/0003-durable-session-storage.md) (Accepted 2026-11-14)
- **Related:** [session model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [testing strategy](testing-strategy.md), [quality gates](quality-gates.md),
  [anti-slop policy](anti-slop-policy.md).

> **Verification status (2026-11-15).** Fase B implemented every capability this plan
> defines: the versioned storage repository, named metadata, reject-only compatibility,
> crash recovery, storage-backed rehydration and the `chronofork session
> save/load/list/delete` CLI. Independent verification ran twice on the same tree.
> Round 1 executed M2-A1–A11 and the CLI contract and found one medium, non-blocking
> finding (F1: the CLI continue-and-save read the latest snapshot before acquiring the
> writer lock). The repair added `SnapshotRepository::with_writer_lock` and
> `save_snapshot_with_lock`; the CLI now acquires the lock before the read and holds it
> through rehydrate/advance/capture/publish. Round 2 re-verified F1 closed with typed
> contention, no stale-base double publish and no regression. Raw evidence lives under
> the ignored paths `.research/m2-durable-sessions/fase-b/verification/` (round 1 plus
> `round2/`), `.research/m2-durable-sessions/fase-b-repair-f1/` and
> `.research/m2-durable-sessions/fase-b-cli/`; the authoritative gate record is the
> dated [quality gates](quality-gates.md) result. Do not cite this document as evidence
> of Windows or network-filesystem durability; those remain unverified.

> **Owner decisions (approved 2026-11-14).** Versioned per-snapshot directory; host
> timestamps advisory only; explicit per-session writer lock/lease; manual list/delete
> retention with no automatic GC; reject incompatible input with no silent migration;
> CLI surface `chronofork session save/load/list/delete` as the contract shape.

## 1. Status rules

1. **`planned - not yet executed`** means the criterion has a proposed test name,
   command and evidence artifact, but no Fase B implementation exists to run.
2. A row becomes **`executed - passed`** only when all of the following are recorded:
   the exact command, exit code, tree commit, test output summary, the ignored
   evidence artifact path, and a verifier review. The authoritative gate record lives
   in [quality gates](quality-gates.md); this document links to it and does not copy
   volatile counts.
3. A row becomes **`executed - failed`** on any mismatch, panic, untyped error or
   missing artifact; a failed row blocks M2. A known failure is never converted into a
   skip, an `#[ignore]`, or a relaxed assertion.
4. A test may be committed before the implementation as a red test only if it fails
   for the expected typed reason. A test that cannot run is not acceptance evidence.
5. No M2 behavior may be described as implemented, working, supported or complete on
   the strength of ADR-0003, this plan, a code review, or a single happy-path run.
6. A planned test whose command cannot execute because its crate or test target does
   not exist is still `planned - not yet executed`; do not rename it "passed" because
   the absent target produced no failure.

As of 2026-11-15 no M2 row remains `planned`; the definitions above are retained for
future criteria.

## 2. Scope, definitions and evidence

- **Storage root:** an explicit caller-supplied directory. There is no default global
  location and no implicit root discovery. The root contract is in
  [ADR-0003 §2](../adr/0003-durable-session-storage.md).
- **Restart:** a second operating-system process opens the same root after the first
  process exits. Criterion M2-A1 is specifically this process boundary.
- **Reopen:** dropping every store, session and backend handle and constructing a new
  store and `SessionManager` over the same root in the same process. Reopen is
  cheaper to test and is used as a complement, never as a replacement for M2-A1.
- **Acceptance backend:** the deterministic synthetic backend
  (`chronofork-test-backend`) is the primary fixture backend because M2 is a storage
  contract, not a hardware-accuracy claim. Repeating M2-A1 and M2-A8 once with the
  minimal Mega Drive slice is desirable but not an M2 gate.
- **Platform:** the durability claim is limited to local filesystems whose directory
  `fsync` and rename behavior is known to be sound (ADR-0003 §4). Linux is the
  acceptance platform for M2. Windows and network filesystems remain unverified
  limitations, not implied passes.
- **Evidence artifacts:** raw output is stored under ignored paths:
  `.research/m2-durable-sessions/fase-b/verification/` (round 1: `SUMMARY.txt`,
  `target-matrix.txt`, `MANIFEST.sha256` and per-criterion transcripts; round 2:
  `round2/SUMMARY.txt`, `round2/MANIFEST.sha256` and `round2/targets/`),
  `.research/m2-durable-sessions/fase-b-repair-f1/` (F1 repair) and
  `.research/m2-durable-sessions/fase-b-cli/` (CLI process transcript). Each artifact
  records the command, tree commit, exit code and the relevant stdout/stderr/SHA-256.
  The artifacts are diagnostic evidence, not durable claims; durable results below are
  summarized without volatile counts.
- **No host time in canonical data:** host timestamps are advisory metadata only.
  Acceptance comparisons use tick, state hash, checkpoint bytes and logical creation
  sequence; they never compare wall-clock times or filesystem modification times.

## 3. M2 exit criteria map

The milestone items are in [milestones](../roadmap/milestones.md). A milestone item
stays unchecked until every criterion mapped to it is `executed - passed`.

| M2 milestone item | Acceptance criteria |
|---|---|
| Checkpoints persist across process restarts | M2-A1, M2-A3, M2-A8, M2-A10 |
| Named snapshots and metadata | M2-A2, M2-A11 |
| Version compatibility policy tested | M2-A4 |
| Concurrency/ownership policy at the service boundary | M2-A5, M2-A6, M2-A7 |
| No mutable state shared across sessions | M2-A3, M2-A9 |
| Minimal CLI contract (owner-approved) | CLI section and its tests |

The nine required M2 exit-criteria cases (a)-(i) map in order to M2-A1 through M2-A9;
M2-A10 and M2-A11 cover crash recovery and explicit retention required by ADR-0003 §13.

**M2 is complete only when** ADR-0003 is accepted, every criterion in this document
is `executed - passed`, and the full repository gate plus the new durable tests pass on
the same tree. Design completion alone does not complete M2.

The executed record satisfies these conditions: every criterion below is
`executed - passed`, ADR-0003 is accepted, and `./scripts/check.sh` exited 0 in both
verification rounds on the same tree. The round-1 CLI finding F1 was repaired and
re-verified closed before this status was recorded.

## 4. Fase B test targets (implemented and executed)

These targets exist and were executed. The implementation stayed in the existing
`chronofork-sessions` crate plus the `chronofork-cli` test target, as planned; no new
crate was needed. Every command below exited 0 in round 2 (per-target output is in
`.research/m2-durable-sessions/fase-b/verification/round2/targets/`); the final test
spelling is recorded there, while the command runs every test in the target. Round 1
executed the same target matrix before the F1 repair.

| Target | Executed command | Criteria | Result |
|---|---|---|---|
| `crates/chronofork-sessions/tests/restart.rs` | `cargo test -p chronofork-sessions --test restart` | M2-A1 (reopen complement), M2-A10 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/metadata.rs` | `cargo test -p chronofork-sessions --test metadata` | M2-A2 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/fork.rs` | `cargo test -p chronofork-sessions --test fork` | M2-A3 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/rejection.rs` | `cargo test -p chronofork-sessions --test rejection` | M2-A4 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/ownership.rs` | `cargo test -p chronofork-sessions --test ownership` | M2-A5 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/confinement.rs` | `cargo test -p chronofork-sessions --test confinement` | M2-A6 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/bounds.rs` | `cargo test -p chronofork-sessions --test bounds` | M2-A7 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/determinism.rs` | `cargo test -p chronofork-sessions --test determinism` and `cargo test -p chronofork-sessions --release --test determinism` | M2-A8 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/isolation.rs` | `cargo test -p chronofork-sessions --test isolation` | M2-A9 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/crash_recovery.rs` | `cargo test -p chronofork-sessions --test crash_recovery` | M2-A10 | exit 0 (round 2) |
| `crates/chronofork-sessions/tests/retention.rs` | `cargo test -p chronofork-sessions --test retention` | M2-A11 | exit 0 (round 2) |
| `crates/chronofork-cli/tests/session_storage.rs` | `cargo test -p chronofork-cli --test session_storage` | M2-A1, M2-A2, M2-A4, M2-A8, CLI contract | exit 0 (round 2) |

The target command runs every test in that file. To prove an individual case during
debugging, append the test function as the filter, for example
`cargo test -p chronofork-sessions --test rejection metadata_checksum_is_rejected`.
The gate records the target command, not only the filtered command.

## 5. Acceptance criteria

Executed record: every target command below ran at tree commit
`3d840634ed21d305762d57513e1da9ff4e22479d` with `CARGO_HOME=/tmp/t7-cargo` in both
verification rounds. All commands exited 0 in round 2, were independently reviewed
(PASS verdict in
`.research/m2-durable-sessions/fase-b/verification/round2/SUMMARY.txt`), and per-target
stdout is in `.research/m2-durable-sessions/fase-b/verification/round2/targets/`.
Round 1 executed the same criteria
(`.research/m2-durable-sessions/fase-b/verification/SUMMARY.txt`,
`.research/m2-durable-sessions/fase-b/verification/target-matrix.txt`); its one finding,
F1, was repaired (`.research/m2-durable-sessions/fase-b-repair-f1/SUMMARY.txt`) and
re-verified closed in round 2. Artifact paths named in each section are relative to
`.research/m2-durable-sessions/fase-b/verification/` unless they name the repair or CLI
directory. The final test spelling is recorded in the per-target artifacts.

### M2-A1 — Restart: a new process reads a committed checkpoint

**Passes only when** process P1 opens a fresh root, creates a session, advances it with
deterministic inputs, captures a checkpoint, publishes a named snapshot, reports its
storage keys, tick and state hash, closes the store and exits; then a separate process
P2 opens the same root, reads the committed session and snapshot, restores the
checkpoint and reports the same tick and state hash. P1 must have exited before P2
starts. No in-process handle may be shared between them. The same process pair also
exercises `save --session`: P2 re-opens the committed session, steps it and appends a
second snapshot under the same session key, and a third process re-reads both snapshots
with increasing `creation_sequence` values.

| Executed target | Command | Result |
|---|---|---|
| Restart/reopen target | `cargo test -p chronofork-sessions --test restart` | exit 0 (round 2; `round2/targets/sessions-restart.out`) |
| CLI process-restart target | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; `round2/targets/cli-session_storage.out`) |
| Manual second-process transcript | `chronofork session save --root R --steps 7 --name baseline`, then a separate process `chronofork session load --root R --session 1` | exit 0 both; `A1-restart-manual.txt` |

**Evidence artifact:** `A1-restart-manual.txt` (both process command lines, exit codes, keys, tick, state hash and the continue-and-save record with unchanged prior checkpoint bytes), plus `round2/targets/sessions-restart.out` and `round2/targets/cli-session_storage.out`.

**Existing precondition:** `Snapshot::to_bytes` and `Snapshot::from_bytes`
(`crates/chronofork-sessions/src/snapshot.rs`) implement the canonical envelope; the
existing `serialized_checkpoint_can_seed_a_new_manager` test in
`crates/chronofork-sessions/tests/acceptance.rs` imports those bytes into a new
manager **in one process**. That covers byte round-trip and import, not a process
restart or filesystem persistence.

**Status:** executed - passed in both rounds: the separate load process reproduces the saved tick and state hash, and continue-and-save appends a snapshot under the same session with the prior checkpoint bytes unchanged.

### M2-A2 — Named snapshot with metadata round-trip

**Passes only when** a snapshot saved with a name, logical creation sequence, lineage
keys and mirrored envelope fields is read back after reopen with the same name, the
same logical sequence, the same lineage and exactly the envelope's backend ID, backend
version, payload format version, tick, configuration hash and state hash. A
metadata-only rename must change only the metadata record; the checkpoint file bytes,
its SHA-256 and the restore state hash must be unchanged. A continue-and-save append
must create a new snapshot under the same session key with a greater
`creation_sequence`, and its name and metadata must round-trip after a later reopen.
Duplicate names are allowed in v1 and must remain labels, not identity authorities.

| Executed target | Command | Result |
|---|---|---|
| Metadata target | `cargo test -p chronofork-sessions --test metadata` | exit 0 (round 2; `round2/targets/sessions-metadata.out`) |
| CLI name round-trip target | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; `round2/targets/cli-session_storage.out`) |
| Manual cross-process metadata transcript | save with `--name`, then load/list in a new process | the name (and the appended snapshot's `creation_sequence`) round-trips across processes (`A1-restart-manual.txt`); metadata-only rename bytes/state-hash is asserted by `rename_preserves_checkpoint_bytes_and_state_hash` in the metadata target, which prints pass lines only |

**Evidence artifact:** `round2/targets/sessions-metadata.out` (metadata/rename assertions),
`round2/targets/cli-session_storage.out` (cross-process CLI name round-trip) and
`A1-restart-manual.txt` (cross-process name and continue-and-save record).

**Pre-Fase-B precondition:** `Snapshot` exposes `id`, `origin_session`,
`format_version`, `backend`, `tick` and `state_hash`; `Lineage` records parent, origin
and depth. Fase B added the versioned metadata record (ADR-0003 §5), which round 2
verifies.

**Status:** executed - passed in both rounds: the metadata target round-trips name, logical creation sequence, lineage and the mirrored envelope fields after reopen; the CLI shows the name across processes; and a metadata-only rename leaves checkpoint bytes and state hash unchanged (asserted by the test, not printed by `cargo test`).

### M2-A3 — Fork after restart preserves lineage, tick and state hash

**Passes only when** a session A and snapshot S are published in one store instance,
all handles are dropped, a fresh store and `SessionManager` open the same root,
A and S are rehydrated, and a fork B from S starts at the same tick and state hash as
S. B must record the durable parent session key for A and the durable origin snapshot
key for S, with the expected depth. Publishing B must use a new storage session key,
must not copy or mutate A's `checkpoint.cfsnap`, and the lineage must survive a second
reopen. A1 supplies the actual cross-process guarantee; A3 is the test that must show
that rehydration through the same validate-then-register path preserves lineage.

| Executed target | Command | Result |
|---|---|---|
| Fork-after-reopen target | `cargo test -p chronofork-sessions --test fork` | exit 0 (round 2; `round2/targets/sessions-fork.out`) |
| Real cross-process boundary | owned by M2-A1; the approved CLI surface has no fork subcommand | `A1-restart-manual.txt` |

**Evidence artifact:** `round2/targets/sessions-fork.out` records the passing target run; the
durable lineage keys, parent-checkpoint byte equality and second-reopen result are
assertions in `fork.rs` (the captured output lists test names and status, not the
asserted values). `A1-restart-manual.txt` supplies the separate real cross-process
boundary.

**Pre-Fase-B precondition:** `SessionManager::fork_from_snapshot` and `Lineage`
implement in-process fork and lineage, and `three_way_fork_is_isolated_and_diverges`
covers in-process fork isolation. Fase B added durable keys, persisted lineage and
storage-backed rehydration.

**Status:** executed - passed in both rounds: fork after reopen preserves lineage, tick and state hash, the child publishes under its own durable key without copying the parent checkpoint, and lineage survives a second reopen. By the accepted plan, A3 is the reopen test; A1 owns the real cross-process boundary.

### M2-A4 — Typed rejection of incompatible, corrupt and identity-mismatched input

**Passes only when** every category below returns the named typed category, without a
panic, without a partial restore, without modifying a valid object, and without a
fallback that guesses missing data or silently migrates. The exact enum spelling is a
Fase B detail; ADR-0003 §6 fixes the category and requires a typed variant.

| Mutated input | Required typed result |
|---|---|
| Root without marker | `NotInitialized` |
| Root marker with wrong magic | `NotAStorageRoot` |
| Unrecognized non-empty directory | `RootNotEmpty` |
| Unsupported storage layout version | `UnsupportedLayoutVersion` with found/supported values |
| Unsupported metadata record version | `UnsupportedMetadataVersion` |
| Metadata magic/checksum/length/trailing-byte corruption | `CorruptMetadata` |
| Metadata record kind mismatched with the file name | `MetadataMismatch` |
| Metadata mirrored envelope field differs from the envelope | `MetadataMismatch` |
| Checkpoint checksum byte flipped | `SessionError::InvalidSnapshot` |
| Checkpoint truncated | `SessionError::InvalidSnapshot` |
| Checkpoint with trailing bytes | `SessionError::InvalidSnapshot` |
| Unsupported outer envelope version | `SessionError::IncompatibleSnapshot` |
| Wrong backend ID, backend version, payload format version or config hash | `SessionError::IncompatibleSnapshot` |
| Checkpoint swapped without its metadata | `MetadataMismatch` (checkpoint SHA-256 binding) |
| Whole snapshot directory swapped | `IdentityMismatch` (directory key vs envelope snapshot ID) |
| Unknown session or snapshot key | `UnknownSession` / `UnknownSnapshot` |
| Future layout directory present | Never read as v1 data, never migrated or adopted; reported as a typed anomaly |

The backend compatibility decision must still come from the existing
`validate_envelope`; storage must not add a second predicate. The outer-envelope
decision must still come from `Snapshot::from_bytes`; storage must not add a second
reader. A malformed metadata record must never authorize guessing fields from a
directory name.

| Executed target | Command | Result |
|---|---|---|
| Rejection target | `cargo test -p chronofork-sessions --test rejection` | exit 0 (round 2; `round2/targets/sessions-rejection.out`) |
| CLI typed-failure target | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; `round2/targets/cli-session_storage.out`) |
| Manual rejection transcript | CLI invocations for missing root, unknown keys, foreign marker, unsupported layout, corrupt metadata and tampered checkpoint | exit 1 typed for each; full-list anomalies exit 0 (`A4-rejection-manual.txt`, `A4-corrupt-anomaly-manual.txt`) |

**Evidence artifact:** `round2/targets/sessions-rejection.out` and `round2/targets/cli-session_storage.out`, with the CLI-level typed transcripts `A4-rejection-manual.txt` and `A4-corrupt-anomaly-manual.txt`.

**Existing precondition:** `crates/chronofork-sessions/src/snapshot.rs` unit tests
(`corruption_is_rejected`, `unsupported_outer_version_is_rejected`,
`truncation_is_rejected`, `arbitrary_bytes_are_rejected_without_panicking`) and
`crates/chronofork-sessions/tests/snapshot_properties.rs` cover the envelope only.
`crates/chronofork-backend-api/src/snapshot.rs` tests cover `validate_envelope`.
`crates/chronofork-sessions/src/manager.rs` test
`importing_a_foreign_snapshot_is_rejected_at_the_session_boundary` covers the
session-boundary translation for a foreign backend snapshot. None of these reads or
validates a storage root, metadata record, lock record or directory key.

**Status:** executed - passed in both rounds: every row of the rejection table returns its named typed category at the repository level or through the CLI, and no repair, migration or fallback path exists.

### M2-A5 — Ownership contention: one writer with typed contention and no hidden global lock

**Passes only when** `writer.lock` is published atomically by hard-linking a complete
private record, so the canonical lock is never visible half-written. A second writer for the
same durable session is rejected immediately with a typed `WriterContended` error
naming the observed owner, with no busy-wait and no silent retry, while a writer for a
different session can proceed concurrently. An empty, partial, unsupported or
otherwise unreadable lock returns `WriterContended` on acquisition and is never
reported as `CorruptLock`; an explicit recovery read of the same unreadable record
returns `CorruptLock` and removes nothing. An expired lease is a diagnostic and is
never auto-stolen; a non-expired lease normally returns `WriterContended`. Explicit
forced recovery still requires the observed owner/generation and the rename-aside
protocol, so a changed record returns `NotLockOwner` without deleting the successor.
If private-temp cleanup fails after the hard-link, acquisition still succeeds; the
leftover temp is a diagnostic, never a lock, and does not revoke the lock. Release or
explicit recovery uses the observed owner token and generation: if the record changed,
it returns `NotLockOwner`, restores or leaves the successor record intact, and deletes
no canonical lock. If the filesystem lacks the hard-link primitive, acquisition fails
closed with `LockPrimitiveUnsupported`. Readers take no writer lock. The only
root-level serialization is the documented short-lived key-allocation lock, never held
during emulation, serialization or checkpoint I/O. The in-process rule stays
`&mut SessionManager` / `&mut Session`; no interior mutability or hidden global lock
may be added.

These cases map to ADR-0003 §6 (`WriterContended`, `NotLockOwner`, `CorruptLock`,
`LockPrimitiveUnsupported`) and §10 (writer-lock publication and owner-checked
rename-aside recovery); the crash-between-publish-and-temp-cleanup and forced-recovery
cases are also required by §13 (lock-record publication; forced recovery with the
owner/generation check).

| Executed target | Command | Result |
|---|---|---|
| Ownership target | `cargo test -p chronofork-sessions --test ownership` | exit 0 (round 2; includes `with_writer_lock_makes_read_modify_publish_linearizable` and `save_snapshot_with_lock_rejects_released_and_foreign_handles`; `round2/targets/sessions-ownership.out`) |
| CLI contention regression | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; `continue_and_save_contends_with_a_process_holding_the_writer_lock`; `round2/targets/cli-session_storage.out`) |
| Cross-process held-lock transcript | helper process holds the real writer lock; real CLI `save --session` | exit 1 typed contention, no snapshot or key reserved; normal append after release (`A5-cross-process-lock.txt`, `round2/F1-manual-contention-and-normal.txt`) |
| Timed real two-process races | repeated CLI `save --session` races | one winner and one typed contention per race, one new snapshot, no stale-base double publish (`round2/F1-real-cli-race-30.txt`, `round2/F1-linearizable-two-process.txt`) |

**Executed case coverage** (assertions in the ownership target; output in
`round2/targets/sessions-ownership.out`):

| Case | Verified behavior |
|---|---|
| F9 crash between hard-link publish and temp cleanup | acquisition succeeds; the leftover `writer.lock.<token>.tmp` is reported as a diagnostic and never treated as a lock; a later release/recovery against the canonical record succeeds |
| F11 forced recovery | a non-expired lease returns `WriterContended`; explicit forced recovery with the matching owner/generation succeeds; forced recovery with a changed record returns `NotLockOwner` and leaves the successor record intact |

**Evidence artifact:** `round2/targets/sessions-ownership.out`, `round2/targets/cli-session_storage.out`, `A5-cross-process-lock.txt`, `A5-cli-race-manual.txt` and the round-2 F1 transcripts `round2/F1-manual-contention-and-normal.txt`, `round2/F1-linearizable-two-process.txt` and `round2/F1-real-cli-race-30.txt`.

**Pre-Fase-B precondition:** `SessionManager` and `Session` mutation require `&mut`
and use `BTreeMap`; `three_way_fork_is_isolated_and_diverges` and
`fork_current_does_not_mutate_the_source` cover in-process isolation. Fase B added the
cross-process writer lock/lease, owner token, atomic lock publication and root
key-allocation lock.

**Status:** executed - passed in both rounds. Round 1 recorded finding F1: the CLI read the latest base before taking the writer lock. The repair added `with_writer_lock`/`save_snapshot_with_lock`; round 2 re-verified that the CLI acquires the lock before the read and holds it through publish, that the loser receives typed contention at acquisition, and that no stale-base double publish is reachable through the CLI. Residual: the lock is cooperative metadata, and the public convenience `save_snapshot` remains publish-only for third-party callers.

### M2-A6 — Path traversal and root confinement are rejected

**Passes only when** every managed path is built from fixed literal components or a
canonical numeric key; no caller-supplied string becomes a path component; the root is
canonicalized once and every managed path stays beneath it. Components equal to `.` or
`..`, containing a path separator or NUL, or using non-canonical numeric text must be
rejected with a typed `PathEscape` before that component is used to build or open a
managed path. Managed symlinks must
be rejected with `SymlinkRejected` and never followed; wrong file types must be
rejected with `WrongFileType`. A snapshot or session name containing `../` or a
separator remains a label and must create no file outside the root. A crafted hostile
tree must produce typed anomalies without reading or writing outside the root.

| Executed target | Command | Result |
|---|---|---|
| Confinement target | `cargo test -p chronofork-sessions --test confinement` | exit 0 (round 2; `round2/targets/sessions-confinement.out`) |
| Manual hostile-tree transcript | symlinked session directory, non-canonical keys, unknown directories and `../` labels | typed anomalies for the hostile components; load through the symlink exits 1; path-like labels remain labels and create nothing outside; outside sentinel unchanged (`A6-hostile-tree-manual.txt`) |

**Evidence artifact:** `round2/targets/sessions-confinement.out` and `A6-hostile-tree-manual.txt` (hostile tree, outside sentinel hash and escape attempt).

**Pre-Fase-B precondition:** there was no storage path code. The shared
`chronofork_core::codec::{Reader, Writer}` is bounds-checked but has no path API.
Fase B added canonical-key parsing and path confinement; Windows reparse-point behavior
remains unverified (ADR-0003 §11) and is not claimed.

**Status:** executed - passed in both rounds: traversal components, non-canonical keys, managed symlinks and wrong file types are rejected or reported as typed anomalies; path-like names (`../…`, `a/b`) remain labels and create nothing outside the root; no file is created or read outside the canonical root. Residual: the checks are path-based and not race-free against a hostile process concurrently replacing a component.

### M2-A7 — Storage bounds: maximum sizes and bounded reads

**Passes only when** every file and listing read is bounded: file length is checked
against the configured limit before allocation, and a bounded reader (the
`take(limit + 1)` pattern) rejects a file that grows after the length check. Oversized
checkpoints, metadata and lock records are rejected with a typed bounds error
(`FileTooLarge`; the exact lock-record variant is pinned by the Fase B plan), an
over-cap list fails with `ListLimitExceeded`, and an over-long or control-character
name fails with `InvalidName`. A list scan over a lowered entry cap must fail rather
than truncate silently. Truncated records remain typed
`InvalidSnapshot`/`CorruptMetadata`; no input may panic or allocate without bound.
Tests may lower the limits through configuration; raising a default requires the ADR
and evidence update described in ADR-0003 §11.

| Executed target | Command | Result |
|---|---|---|
| Bounds target | `cargo test -p chronofork-sessions --test bounds` | exit 0 (round 2; `round2/targets/sessions-bounds.out`) |
| Structural inspection | length-before-allocation and bounded-read audit | `structural-as01-as02.txt` (round 1; unchanged code paths in round 2) |

**Evidence artifact:** `round2/targets/sessions-bounds.out` and the length-before-allocation/bounded-read inspection in `structural-as01-as02.txt`.

**Pre-Fase-B precondition:** `chronofork_core::codec` provides bounds-checked reading
and writing; `Snapshot::from_bytes` rejects short input and trailing bytes. Fase B added
the file-length caps, growth check, list cap, name bounds and lock-record bounds that
round 2 verifies.

**Status:** executed - passed in both rounds: oversized, growing, truncated and over-long inputs are rejected with typed errors; lengths are validated before allocation and reads are bounded.

### M2-A8 — Determinism: the same checkpoint reproduces the same state hash after restart

**Passes only when** `checkpoint.cfsnap` is byte-for-byte the output of
`Snapshot::to_bytes()`, and restoring it after restart yields the same tick and state
hash as before the restart. Re-capturing after restore must reproduce the identical
canonical bytes and state hash. Changing a name, logical creation sequence or advisory
host timestamp must not change checkpoint bytes, the state hash or a replay result, and
host time must never order or validate snapshots. Two independent restarts and a
debug/release pair must produce identical reports. No wall-clock time, filesystem
modification time, process ID or entropy may enter the checkpoint or state hash.

| Executed target | Command | Result |
|---|---|---|
| Determinism target (debug) | `cargo test -p chronofork-sessions --test determinism` | exit 0 (round 2; `round2/targets/sessions-determinism.out`) |
| Determinism target (release) | `cargo test -p chronofork-sessions --release --test determinism` | exit 0 (round 2; `round2/targets/sessions-determinism-release.out`) |
| Manual determinism transcript | two roots created about 1.2 s apart; helper compares stored bytes with `Snapshot::to_bytes`; debug and release CLI runs | identical checkpoint SHA-256 and state hash; byte-equal envelope; `created_at` only in metadata (`A8-determinism-manual.txt`) |

Debug/release equality is compared **across the two recorded artifacts**, not inside one
test process: run the target in both modes and compare the reports field by field.

**Evidence artifact:** `round2/targets/sessions-determinism.out`, `round2/targets/sessions-determinism-release.out` and `A8-determinism-manual.txt` (byte-equality with `Snapshot::to_bytes`, cross-root and debug/release hashes, `created_at` only in metadata).

**Existing precondition:** `Snapshot::to_bytes` is the canonical serializer and the
existing `serialized_checkpoints_round_trip_exactly` property test covers byte
round-trip in memory; `restore_and_replay_reproduce_the_branch_state` covers
restore-plus-replay in one process. Neither covers filesystem bytes, restart or
metadata variation.

**Status:** executed - passed in both rounds: identical inputs reproduce byte-identical checkpoints, tick and state hash across roots, processes and debug/release builds; host timestamps never enter canonical bytes, ordering or validation.

### M2-A9 — No mutable state shared across sessions, including persistence

**Passes only when** each rehydrated session owns its own mutable backend and
checkpoints; forking never shares mutable state with the source; publishing or deleting
one session or snapshot never changes another session's checkpoint bytes, state hash,
lineage or list ordering; independent readers see independent results; and storage adds
no hidden global mutable registry. The in-process ownership rules stay explicit:
mutation requires `&mut`, and shared access at a future service boundary needs its own
documented synchronization policy.

| Executed target | Command | Result |
|---|---|---|
| Isolation target | `cargo test -p chronofork-sessions --test isolation` | exit 0 (round 2; `round2/targets/sessions-isolation.out`) |

**Evidence artifact:** `round2/targets/sessions-isolation.out`.

**Pre-Fase-B precondition:** `three_way_fork_is_isolated_and_diverges` and
`fork_current_does_not_mutate_the_source` in
`crates/chronofork-sessions/tests/acceptance.rs` cover in-process state and memory
isolation; `SessionManager` uses `&mut` and an ordered `BTreeMap`. Fase B added the
durable session/storage-level isolation tests.

**Status:** executed - passed in both rounds: rehydrated sessions own independent mutable state, forks never share it, and publishing or deleting one session/snapshot never changes another checkpoint, state hash or lineage.

### M2-A10 — Crash/fault recovery: staging and tombstones are never committed data

**Passes only when** a `.staging-*` directory is never loaded, a `.trash-*` tombstone is
reported and never resurrected, a missing or corrupt checkpoint/metadata record is a
typed anomaly rather than a repair or silent skip, and an `fsync` failure before the
commit rename aborts unpublished with a typed I/O error. A `sync_all` failure on the
parent directory after the commit rename must return typed `DurabilityUnconfirmed`: the
object may be visible, but the publish is neither durable nor successful, success is
never reported, the writer lock is released before the error, and the caller must
re-open/validate or reconcile by retrying the parent sync and re-validating; it must not
blindly retry the publish because the key may exist. The same post-rename rule applies
to metadata-only replacement and explicit delete. Reconciliation must distinguish
three outcomes: visible-confirmed when a retried parent sync succeeds and re-validation
matches; visible-unconfirmed when the object remains visible but no durable
confirmation exists; and lost when the object disappeared before reconciliation and
re-open reports the loss. No outcome may be reported as confirmed success without a
successful parent sync and validation.

Key and creation-sequence reservations advance past a crash so a value is never reused.
A missing, short, oversized or unreadable `state/session-keys` returns
`CorruptAllocationCounter` and fails closed; it is never reset to one automatically. A
persisted counter gap is never reused. A stale `state/allocation.lock` is never
auto-stolen; its explicit recovery uses the same owner+generation rename-aside protocol
with operator confirmation, clears the lock without changing the persisted counter,
returns `NotLockOwner` for a changed record and `CorruptLock` for an unreadable record
without deleting it. An unconfirmed allocation counter update returns
`DurabilityUnconfirmed` and publishes nothing. Opening, capturing, closing or exiting
must not delete committed data. A failed publish may remove only its own staging
directory.

The Fase B plan must choose the test mechanism: (i) construct the documented partial
on-disk trees directly for recovery tests, and (ii) add a test-only I/O failure seam for
`fsync`/rename failures. If (ii) is rejected as speculative, every publish step it
would have covered must be listed as an unverified limitation instead of being claimed
as tested. A real power-loss test is not portable and is not required.

These cases map to ADR-0003 §4 (post-rename `DurabilityUnconfirmed` and reconciliation
outcomes), §6 (typed durability/concurrency/allocation categories), §10
(allocation-lock and counter rules) and §13 (reconciliation outcomes; corrupt/missing
`state/session-keys` and counter gaps).

| Executed target | Command | Result |
|---|---|---|
| Crash-recovery target | `cargo test -p chronofork-sessions --test crash_recovery` | exit 0 (round 2; Unix-gated, uid 1000; `round2/targets/sessions-crash_recovery.out`) |

**Executed case coverage** (assertions in the crash-recovery target; output in
`round2/targets/sessions-crash_recovery.out`):

| Case | Verified behavior |
|---|---|
| F10(a) retried parent sync confirms | object remains visible; retried parent sync succeeds; re-open/validation matches; outcome recorded as visible-confirmed |
| F10(b) still unconfirmed | object remains visible; parent sync still fails or is unconfirmed; no success is reported; outcome recorded as visible-unconfirmed |
| F10(c) lost before reconciliation | object disappeared before retry; re-open reports the loss; no resurrection or automatic repair |
| F12 corrupt/missing session-keys | missing, short, oversized and unreadable `state/session-keys` each return `CorruptAllocationCounter`; counter is not reset to one |
| F12 persisted-counter gap | gap is persisted; reserved key and every earlier gap key are never reused; next minted key is greater than every published key |

**Evidence artifact:** `round2/targets/sessions-crash_recovery.out` (Unix-gated permission fault injection; each staging/tombstone/sync/recovery case).

**Pre-Fase-B precondition:** none. Fase B added the publish sequence, staging
directory, tombstone, fsync and lock publication/recovery paths that these
fault-injection tests exercise.

**Status:** executed - passed in both rounds with real permission-fault injection: staging and tombstone objects are never committed, failed publishes never advance visible state, post-rename sync failures return typed `DurabilityUnconfirmed`, and allocation recovery is explicit and never reuses persisted gaps. Scope: injected filesystem faults, not power loss; crash recovery is Unix-gated and Windows durability is unverified.

### M2-A11 — Explicit retention: deterministic list/delete and no automatic GC

**Passes only when** `list` returns only entries that pass the full validation order in
its normal result, orders sessions by storage session key and snapshots by
`creation_sequence` then key, reports invalid entries as typed anomalies instead of
including or silently skipping them, and never repairs or migrates anything. Opening, capturing a checkpoint, closing a session and
process exit delete no committed data. Deleting requires an explicit call under the
session writer lock, removes only the target via the visibility-removing rename, never
resurrects a tombstone, does not cascade to children and does not rewrite another
session's lineage. Deleting a session that still has snapshots is rejected until the
snapshots are explicitly deleted. Publishing with an existing key is a typed conflict,
never an overwrite. A list scan over the configured cap fails with `ListLimitExceeded`.

| Executed target | Command | Result |
|---|---|---|
| Retention target | `cargo test -p chronofork-sessions --test retention` | exit 0 (round 2; `round2/targets/sessions-retention.out`) |
| Restart/list manual evidence | `list` twice, save/load in separate processes | deterministic listing; open/capture/close/restart delete nothing (`A1-restart-manual.txt`) |

**Evidence artifact:** `round2/targets/sessions-retention.out` and the list/restart record in `A1-restart-manual.txt`.

**Pre-Fase-B precondition:** none. Before Fase B, `chronofork-sessions` performed no
filesystem I/O and the CLI wrote only trace/state output; the `session` commands did not
exist. Fase B added the retention surface and the `chronofork session` CLI;
`FR-SNAPSHOT-006` is Implemented in the [functional
requirements](../requirements/functional.md).

**Status:** executed - passed in both rounds: listing is deterministic, retention is caller-driven, deletion is explicit and non-cascading, an existing key is a conflict rather than an overwrite, and no operation performs automatic garbage collection.

## 6. Owner-approved minimal CLI surface (implemented and verified)

The CLI at `crates/chronofork-cli/src/main.rs` implements this surface and documents
it in `chronofork help`. Round 1 verification found that `save --session` read the
latest base before taking the writer lock (F1); the repaired
implementation acquires the per-session writer lock before the read and holds it
through rehydrate/advance/capture/publish via `SnapshotRepository::with_writer_lock`
and `save_snapshot_with_lock`, and round 2 re-verified typed contention, no stale-base
double publish and no regression. The CLI still performs no storage, serialization,
compatibility or lock logic itself.

The owner-approved M2 contract is deliberately small, std-only and line-oriented. It is
the process boundary used by the M2-A1 restart test. The command names and flags below
were approved on 2026-11-14; the semantics remain the contract shape for Fase B.

```text
chronofork session save --root <DIR> [--session <KEY>] [--name <NAME>]
                        [--steps <N>] [--input <U32>]
chronofork session load --root <DIR> --session <KEY> [--snapshot <KEY>]
                        [--steps <N>] [--input <U32>]
chronofork session list --root <DIR> [--session <KEY>]
chronofork session delete --root <DIR> --session <KEY> [--snapshot <KEY>]
```

Behavior contract:

- **`--root` is required for every session command.** There is no default root, no
  environment-variable root and no implicit creation. `save` may initialize an empty
  directory or an already-initialized root; `load`, `list` and `delete` must not
  initialize a root and must return the typed `NotInitialized` error when the marker is
  absent.
- **`save` without `--session`** creates a fresh deterministic synthetic-backend
  session, applies `--input` for `--steps` steps (default `0` steps, neutral input),
  captures the checkpoint, publishes it as the session's first named snapshot and
  prints its keys and state summary. Each such invocation creates a new durable session.
- **`save --session <KEY>`** is the continue-and-save workflow. It rehydrates that
  session's latest committed snapshot, applies `--steps` with `--input` to the
  in-memory instance (default `0` steps), captures a new checkpoint and publishes a new
  snapshot under the same durable session. Expected writes: no new durable session key;
  exactly one new snapshot key with a `creation_sequence` greater than the previous
  maximum; the existing snapshots' `checkpoint.cfsnap` bytes remain unchanged. It holds
  the session writer lock for the read-validate-publish sequence and releases it after
  publishing, then prints the session block followed by the new snapshot block.
  Implementation note: the lock is acquired before `latest_snapshot`/`load_snapshot`
  and released on every success and error path; a second writer receives typed
  `WriterContended` at acquisition (F1, found in round 1 and closed in round 2).
- **`load`** is a read operation: it rehydrates and prints the selected checkpoint
  without writing storage. `--steps`/`--input` advance only the in-memory instance and
  do not persist. `--snapshot` defaults to the latest committed snapshot by
  `creation_sequence`; an unknown session or snapshot is a typed error.
- **`list`** is a read operation. It prints one session block followed by that
  session's snapshot blocks, in the deterministic order from ADR-0003 §9. It reports
  typed anomalies on stderr and never repairs them.
- **`delete`** is explicit: `--snapshot` deletes one snapshot; without it, deletes an
  empty session. Deleting a session that still has snapshots fails; deletion never
  cascades and never runs automatically.
- **`--name`** is optional, bounded UTF-8, non-unique and never a path component; it is
  rejected with `InvalidName` for control characters or over-long input.

Output contract (stable, machine-parseable, stdout only). Each record is a block of
`key=value` lines; blocks are separated by one blank line. The first field is
`record=session`, `record=snapshot` or `record=deleted`. `save` prints a session block
followed by its new snapshot block; `load` prints one snapshot block; `list` prints one
session block followed by its snapshot blocks, in the deterministic order above;
`delete` prints one `record=deleted` block with the deleted `session_key` and, when
applicable, `snapshot_key`. Only fields that apply to that record kind are printed:

```text
record=session
session_key=1
name=baseline

record=snapshot
session_key=1
snapshot_key=1
name=baseline
tick=15
state_hash=<hex>
creation_sequence=1
parent_session_key=-
origin_snapshot_key=-
```

A consumer splits each line at its first `=`; names may contain spaces and `=` but no
newline or other control character (`InvalidName` rejects them), so the block structure
is unambiguous. `name=` is empty when unnamed; lineage fields use `-` when absent;
`state_hash` is lowercase hexadecimal. There is no `--json` flag in the M2 CLI contract:
adding JSON would need a serializer decision and is out of scope.

Exit and stream contract: `0` on success and `1` on any typed failure; stdout carries
only the data format above; stderr carries `chronofork: <error>` diagnostics; no
diagnostics contaminate stdout. There is no network access and no telemetry.

CLI acceptance tests and their conditions:

| Executed test | Command | Result |
|---|---|---|
| CLI session contract target (all §6 cases) | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; `round2/targets/cli-session_storage.out`) |
| F1 contention regression: `continue_and_save_contends_with_a_process_holding_the_writer_lock` | `cargo test -p chronofork-cli --test session_storage` | exit 0 (round 2; typed contention, no snapshot or key reserved) |
| Manual F1 contention and normal path | one holder process plus real CLI `save --session` | typed contention before any read; normal append after release (`round2/F1-manual-contention-and-normal.txt`) |
| Manual lock-aware two-process sequence | two processes run the CLI sequence | one publish; loser receives typed contention; no stale-base publish (`round2/F1-linearizable-two-process.txt`) |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b-cli/CLI-session.txt` with
each CLI invocation, exit code, stdout/stderr split, parsed keys and state hashes; the
continue-and-save record shows the same session key, one new snapshot key with a greater
`creation_sequence`, and unchanged bytes for the prior checkpoint. Round-2 contention
and race transcripts are in
`.research/m2-durable-sessions/fase-b/verification/round2/` (`F1-*.txt`), with
per-target stdout in
`.research/m2-durable-sessions/fase-b/verification/round2/targets/`.

**Status of the whole CLI surface:** executed - passed in round 2. Round 1 passed the
contract tests but recorded finding F1 (the writer lock did not cover the read); the
repair and round-2 re-verification closed it without changing the approved contract
text. Residual limits: the lock is cooperative, the CLI holds it across any in-memory
`--steps` advance (long advances extend contention), and the public convenience
`save_snapshot` remains publish-only for third-party callers.

## 7. Existing baseline: what exists and what it does not cover

These tests exist today and may be used as preconditions. They do **not** satisfy any
durable criterion. Run them rather than trusting this summary; their authoritative
results belong in [quality gates](quality-gates.md).

| Existing evidence | What it is designed to show | What it does not cover |
|---|---|---|
| `crates/chronofork-sessions/src/snapshot.rs` tests (`snapshot_round_trips_through_bytes`, `corruption_is_rejected`, `unsupported_outer_version_is_rejected`, `truncation_is_rejected`, `arbitrary_bytes_are_rejected_without_panicking`) | The canonical envelope round-trips and rejects corruption/version/truncation in memory | File bytes, restart, metadata, storage root |
| `crates/chronofork-sessions/tests/snapshot_properties.rs` | Property coverage for round-trip, truncation, single-byte corruption and appended bytes | Any filesystem or storage behavior |
| `crates/chronofork-sessions/tests/acceptance.rs::serialized_checkpoint_can_seed_a_new_manager` | Serialized bytes can import into a new manager in one process | Process restart, storage persistence, durable identity |
| `crates/chronofork-sessions/tests/acceptance.rs::restore_and_replay_reproduce_the_branch_state` | Restore plus replay reproduces tick and state hash in one process | Restart or durable checkpoint bytes |
| `crates/chronofork-sessions/tests/acceptance.rs::three_way_fork_is_isolated_and_diverges` and `fork_current_does_not_mutate_the_source` | In-process fork isolation, lineage and source non-mutation | Durable lineage, storage-backed rehydration, persistence ownership |
| `crates/chronofork-sessions/src/manager.rs::importing_a_foreign_snapshot_is_rejected_at_the_session_boundary` | Backend incompatibility is translated to a session-level typed error | Storage metadata/binding and root validation |
| `crates/chronofork-backend-api/src/snapshot.rs` tests | `validate_envelope` rejects each compatibility field by name | Storage-level compatibility matrix |
| `crates/chronofork-core/src/codec.rs` | Bounds-checked reader/writer primitives | File-length caps, bounded file reads, list caps |
| `crates/chronofork-cli/tests/demo.rs` and `trace.rs` | CLI process-level test pattern, temp directories, deterministic output | A `session` subcommand or storage root; the Fase B `session_storage` target in §4 covers that surface |

The Fase B targets listed in §4 are separate: they do satisfy the durable criteria and
are recorded as `executed - passed`. The commands below were recorded during Fase B
execution:

```sh
cargo test -p chronofork-sessions --test acceptance
cargo test -p chronofork-sessions --test snapshot_properties
cargo test -p chronofork-sessions --lib
cargo test -p chronofork-backend-api --lib
cargo test -p chronofork-cli
```

| Precondition command | Recorded result |
|---|---|
| `cargo test -p chronofork-sessions --test acceptance` | exit 0; 5 passed, 0 failed |
| `cargo test -p chronofork-sessions --test snapshot_properties` | exit 0; 5 passed, 0 failed |
| `cargo test -p chronofork-sessions --lib` | exit 0; 25 passed, 0 failed |
| `cargo test -p chronofork-backend-api --lib` | exit 0; 3 passed, 0 failed |
| `cargo test -p chronofork-cli` | exit 0; 10 passed, 0 failed |

Recorded by the independent t6 verification in
`.research/m2-durable-sessions/fase-b/verification/round3/precondition-commands.txt`
(HEAD `3d84063`). The pass counts are that recorded-run's values, not current claims.

## 8. Traceability

| Criterion | Requirements | ADR-0003 sections |
|---|---|---|
| M2-A1 restart | FR-SNAPSHOT-006, NFR-DUR-001 | §2-4, §13 |
| M2-A2 metadata | FR-SESSION-006, FR-SNAPSHOT-006, NFR-DUR-005 | §5 |
| M2-A3 fork after restart | FR-FORK-001..005, NFR-ISO-001..003, NFR-DUR-001 | §8 |
| M2-A4 typed rejection | FR-SNAPSHOT-003, NFR-DUR-003/004, NFR-ISO-005 | §6-7 |
| M2-A5 ownership and lock recovery | FR-SESSION-006, NFR-CONC-001/002 | §6, §10, §13 |
| M2-A6 confinement | NFR-STOR-001 | §11 |
| M2-A7 bounds | NFR-STOR-002 | §11 |
| M2-A8 determinism | NFR-DET-003/004, NFR-DUR-001/005 | §3, §5 |
| M2-A9 isolation | FR-FORK-005, NFR-ISO-001..003 | §8-10 |
| M2-A10 crash, fsync, reconciliation and allocation recovery | NFR-DUR-002, NFR-STOR-003/004 | §4, §6, §9-10, §13 |
| M2-A11 retention | FR-SNAPSHOT-006, NFR-STOR-003/004 | §9 |
| CLI contract | FR-SESSION-006, FR-SNAPSHOT-006 | §2, §9-10 |

`NFR-CONC-003` is not mapped to an M2-A criterion and remains **Partial**: the
lock-free-reader tests are sequential (including a reader while a writer lock is held
but no publish is in progress), and the concurrent publish/read and delete/read
interleavings are not separately tested (see the requirements and §10). M2 completion
rests on the executed M2-A rows, not on that NFR clause.

## 9. Execution record (2026-11-15)

1. Fase B was implemented under a separately approved plan in the existing
   `chronofork-sessions` and `chronofork-cli` crates; no new crate was needed.
2. Storage was implemented without a second snapshot serializer, reader or
   compatibility predicate, reusing `Snapshot::to_bytes`/`from_bytes`,
   `validate_envelope` and the shared `chronofork_core::codec`.
3. Round 1 executed every target, the precondition commands and the full repository
   gate. Raw evidence: `.research/m2-durable-sessions/fase-b/verification/`
   (`SUMMARY.txt`, `target-matrix.txt`, `MANIFEST.sha256` and per-criterion
   transcripts). The verdict was PASS for M2-A1–A11 with one medium, non-blocking CLI
   lock-scope finding (F1).
4. The F1 repair added `SnapshotRepository::with_writer_lock` and
   `save_snapshot_with_lock`; the CLI `save --session` path acquires the lock before
   the read and holds it through publish. Repair evidence:
   `.research/m2-durable-sessions/fase-b-repair-f1/SUMMARY.txt`.
5. Round 2 independently re-ran the gate and every target on the repaired tree, closed
   F1 and found no regression
   (`.research/m2-durable-sessions/fase-b/verification/round2/SUMMARY.txt`,
   `MANIFEST.sha256`, `targets/`). The authoritative gate result is recorded in
   [quality gates](quality-gates.md).
6. Documentation was promoted from this record without copying volatile test counts;
   the verifier remains the owner of the raw verification evidence.

## 10. Out of scope and stated limitations

- This document records an executed implementation; it is not itself the raw
  verification evidence. Raw artifacts remain under the ignored `.research/` paths.
- No claim of Windows or network-filesystem durability; directory `fsync`/rename
  behavior there is unverified (ADR-0003 §4).
- No encryption, authentication, redaction, deduplication, compression or
  multi-tenant isolation; the writer lock is cooperative, not a security boundary. A
  long in-memory `--steps` advance under `save --session` extends lock hold time by
  design, and a crashed holder needs explicit owner/generation-checked recovery.
- No automatic retention, garbage collection, background repair or silent migration in
  v1; retention is caller-driven and deletion is explicit.
- No distributed lock or multi-writer support. The public convenience
  `save_snapshot` acquires its lock only at publish, so a third-party caller that reads
  a base first can recreate the stale-base window; the CLI uses the lock-aware
  `with_writer_lock` workflow. No `unsafe`; no new dependency was added.
- `NFR-CONC-003` is **Partial**: lock-free readers are tested sequentially, including a
  reader while a writer lock is held but no publish is in progress. A concurrent
  publish/read or delete/read interleaving is not exercised by any test; those clauses
  rely on the immutable published object and the atomic visibility-removing rename and
  remain unverified. This NFR clause is not an M2-A exit criterion.
- No performance, capacity or throughput target is claimed for M2.
- No real-hardware compatibility claim; the acceptance fixture is the deterministic
  synthetic backend, with an optional Mega Drive repetition.

## 11. Change control

- A change to a criterion, expected typed category, command, target name or artifact
  path requires an explicit ADR/plan amendment; it must not be made by quietly
  renaming a test or loosening an assertion.
- The names in this document describe the approved contract shape. Final spelling may
  be refined by the approved Fase B plan; the semantics and rejection boundaries must
  remain equivalent.
- If a planned command cannot run because the target is absent, the row remains
  `planned - not yet executed`; the gap is reported, not hidden.
- Once a row has executed, its result is append-only evidence linked from
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure. The
  round-1 finding F1 and its round-2 closure are recorded, not erased.
