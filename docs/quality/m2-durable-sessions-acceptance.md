# M2 Durable Sessions — Executable Acceptance Plan

- **Status:** Fase A accepted design and acceptance plan. Every acceptance row below
  is **planned - not yet executed**.
- **Date:** 2026-11-14
- **Companion contract:** [ADR-0003: durable session
  storage](../adr/0003-durable-session-storage.md) (Accepted 2026-11-14)
- **Related:** [session model](../architecture/session-model.md), [functional
  requirements](../requirements/functional.md), [non-functional
  requirements](../requirements/non-functional.md), [milestones](../roadmap/milestones.md),
  [testing strategy](testing-strategy.md), [quality gates](quality-gates.md),
  [anti-slop policy](anti-slop-policy.md).

> **Implementation status.** Nothing in this document has been executed. The current
> tree has no durable storage, no named metadata record, no durable writer lock, no
> storage-backed rehydration path and no `chronofork session` CLI command. ADR-0003 is
> Accepted (2026-11-14); no implementation exists. This document is an acceptance plan,
> not evidence that any M2 capability exists; do not cite it as a pass result.

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
- **Evidence artifacts:** raw output is stored under the ignored
  `.research/m2-durable-sessions/fase-b/` path (one file per criterion). Each artifact
  records the command, tree commit, UTC start time, exit code, full stdout/stderr
  summary and every relevant SHA-256. The artifact is diagnostic evidence, not a
  durable claim; durable results are summarized without volatile counts.
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

## 4. Planned Fase B test targets

These target names are contract proposals, not existing files. The default placement
is the existing `chronofork-sessions` crate: the M2 owner direction puts the snapshot
repository there, and ADR-0003 proposes no new crate for layout v1. ADR-0003 §14
currently records no crate-placement owner decision, so adding a new crate or moving
these targets requires an explicit owner decision and an approved Fase B plan. If such
a plan moves them, change only the `-p` package and path in the commands below. Test
names, semantics and evidence requirements stay unchanged.

| Planned target | Planned target command | Criteria |
|---|---|---|
| `crates/chronofork-sessions/tests/restart.rs` | `cargo test -p chronofork-sessions --test restart` | M2-A1 (reopen complement), M2-A10 |
| `crates/chronofork-sessions/tests/metadata.rs` | `cargo test -p chronofork-sessions --test metadata` | M2-A2 |
| `crates/chronofork-sessions/tests/fork.rs` | `cargo test -p chronofork-sessions --test fork` | M2-A3 |
| `crates/chronofork-sessions/tests/rejection.rs` | `cargo test -p chronofork-sessions --test rejection` | M2-A4 |
| `crates/chronofork-sessions/tests/ownership.rs` | `cargo test -p chronofork-sessions --test ownership` | M2-A5 |
| `crates/chronofork-sessions/tests/confinement.rs` | `cargo test -p chronofork-sessions --test confinement` | M2-A6 |
| `crates/chronofork-sessions/tests/bounds.rs` | `cargo test -p chronofork-sessions --test bounds` | M2-A7 |
| `crates/chronofork-sessions/tests/determinism.rs` | `cargo test -p chronofork-sessions --test determinism` and `cargo test -p chronofork-sessions --release --test determinism` | M2-A8 |
| `crates/chronofork-sessions/tests/isolation.rs` | `cargo test -p chronofork-sessions --test isolation` | M2-A9 |
| `crates/chronofork-sessions/tests/crash_recovery.rs` | `cargo test -p chronofork-sessions --test crash_recovery` | M2-A10 |
| `crates/chronofork-sessions/tests/retention.rs` | `cargo test -p chronofork-sessions --test retention` | M2-A11 |
| `crates/chronofork-cli/tests/session_storage.rs` | `cargo test -p chronofork-cli --test session_storage` | M2-A1, M2-A2, M2-A4, M2-A8, CLI contract |

The target command runs every test in that file. To prove an individual case during
debugging, append the test function as the filter, for example
`cargo test -p chronofork-sessions --test rejection metadata_checksum_is_rejected`.
The gate records the target command, not only the filtered command.

## 5. Acceptance criteria

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

| Planned test | Planned command |
|---|---|
| `restart.rs::reopen_after_writer_close_round_trips` | `cargo test -p chronofork-sessions --test restart` |
| `session_storage.rs::save_then_load_in_a_new_process_round_trips` | `cargo test -p chronofork-cli --test session_storage` |
| `restart.rs::no_partial_object_is_visible_before_publish` | `cargo test -p chronofork-sessions --test restart` |
| `session_storage.rs::save_session_continue_and_save_appends_snapshot` | `cargo test -p chronofork-cli --test session_storage` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A1-restart.txt` with both
process command lines, exit codes, reported keys, tick and state hash, plus the
continue-and-save invocation (same session key, new snapshot key and creation
sequence, prior checkpoint bytes unchanged).

**Existing precondition:** `Snapshot::to_bytes` and `Snapshot::from_bytes`
(`crates/chronofork-sessions/src/snapshot.rs`) implement the canonical envelope; the
existing `serialized_checkpoint_can_seed_a_new_manager` test in
`crates/chronofork-sessions/tests/acceptance.rs` imports those bytes into a new
manager **in one process**. That covers byte round-trip and import, not a process
restart or filesystem persistence.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `metadata.rs::named_snapshot_round_trips_metadata` | `cargo test -p chronofork-sessions --test metadata` |
| `metadata.rs::rename_preserves_checkpoint_bytes_and_state_hash` | `cargo test -p chronofork-sessions --test metadata` |
| `metadata.rs::duplicate_names_are_labels_not_identities` | `cargo test -p chronofork-sessions --test metadata` |
| `metadata.rs::continue_and_save_appended_snapshot_metadata_round_trips` | `cargo test -p chronofork-sessions --test metadata` |
| `session_storage.rs::save_name_is_visible_after_new_process` | `cargo test -p chronofork-cli --test session_storage` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A2-metadata.txt` with the
before/after checkpoint SHA-256, state hash, metadata field dump, list output and the
appended snapshot metadata from `save --session`.

**Existing precondition:** `Snapshot` exposes `id`, `origin_session`, `format_version`,
`backend`, `tick` and `state_hash`; `Lineage` records parent, origin and depth. There
is no metadata record format or name field in the current tree.

**Status:** planned - not yet executed.

### M2-A3 — Fork after restart preserves lineage, tick and state hash

**Passes only when** a session A and snapshot S are published in one store instance,
all handles are dropped, a fresh store and `SessionManager` open the same root,
A and S are rehydrated, and a fork B from S starts at the same tick and state hash as
S. B must record the durable parent session key for A and the durable origin snapshot
key for S, with the expected depth. Publishing B must use a new storage session key,
must not copy or mutate A's `checkpoint.cfsnap`, and the lineage must survive a second
reopen. A1 supplies the actual cross-process guarantee; A3 is the test that must show
that rehydration through the same validate-then-register path preserves lineage.

| Planned test | Planned command |
|---|---|
| `fork.rs::fork_after_reopen_preserves_lineage_tick_and_hash` | `cargo test -p chronofork-sessions --test fork` |
| `fork.rs::fork_publishes_under_child_session_key_without_copying_parent_checkpoint` | `cargo test -p chronofork-sessions --test fork` |
| `fork.rs::lineage_survives_a_second_reopen` | `cargo test -p chronofork-sessions --test fork` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A3-fork.txt` with the
parent/child durable keys, lineage values, ticks, state hashes and both checkpoint
SHA-256 values.

**Existing precondition:** `SessionManager::fork_from_snapshot` and `Lineage` implement
in-process fork and lineage, and `three_way_fork_is_isolated_and_diverges` covers
in-process fork isolation. No durable keys, storage-backed rehydration or persisted
lineage exist.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `rejection.rs::root_and_layout_rejections_are_typed` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::metadata_corruption_and_version_rejections_are_typed` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::checkpoint_corruption_truncation_and_trailing_bytes_are_typed` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::backend_compatibility_fields_are_rejected_once` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::identity_and_metadata_binding_rejections_are_typed` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::unknown_keys_are_typed_and_do_not_create_state` | `cargo test -p chronofork-sessions --test rejection` |
| `rejection.rs::arbitrary_record_bytes_never_panic` | `cargo test -p chronofork-sessions --test rejection` |
| `session_storage.rs::unknown_session_exits_one_with_typed_diagnostic` | `cargo test -p chronofork-cli --test session_storage` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A4-rejection.txt` with one
line per case: mutation, expected category, observed category and exit code.

**Existing precondition:** `crates/chronofork-sessions/src/snapshot.rs` unit tests
(`corruption_is_rejected`, `unsupported_outer_version_is_rejected`,
`truncation_is_rejected`, `arbitrary_bytes_are_rejected_without_panicking`) and
`crates/chronofork-sessions/tests/snapshot_properties.rs` cover the envelope only.
`crates/chronofork-backend-api/src/snapshot.rs` tests cover `validate_envelope`.
`crates/chronofork-sessions/src/manager.rs` test
`importing_a_foreign_snapshot_is_rejected_at_the_session_boundary` covers the
session-boundary translation for a foreign backend snapshot. None of these reads or
validates a storage root, metadata record, lock record or directory key.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `ownership.rs::second_writer_is_rejected_with_typed_contention` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::half_written_or_empty_writer_lock_is_writer_contended_on_acquisition` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::crash_between_lock_publish_and_temp_cleanup_keeps_lock_and_reports_diagnostic` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::forced_recovery_with_owner_generation_succeeds_and_changed_owner_returns_not_lock_owner` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::release_or_recovery_with_changed_owner_keeps_successor_and_returns_not_lock_owner` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::unreadable_lock_on_explicit_recovery_is_corrupt_lock_and_removes_nothing` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::hard_link_unsupported_fails_closed_with_lock_primitive_unsupported` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::distinct_sessions_write_concurrently_without_a_global_lock` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::expired_lease_is_not_auto_stolen` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::key_allocation_lock_is_short_lived_and_root_scoped` | `cargo test -p chronofork-sessions --test ownership` |
| `ownership.rs::lock_free_reader_never_observes_a_torn_checkpoint` | `cargo test -p chronofork-sessions --test ownership` |

**Planned evidence rows in A5-ownership.txt:**

| Case | Row content |
|---|---|
| F9 crash between hard-link publish and temp cleanup | acquisition succeeds; the leftover `writer.lock.<token>.tmp` is reported as a diagnostic and never treated as a lock; a later release/recovery against the canonical record succeeds |
| F11 forced recovery | a non-expired lease returns `WriterContended`; explicit forced recovery with the matching owner/generation succeeds; forced recovery with a changed record returns `NotLockOwner` and leaves the successor record intact |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A5-ownership.txt` with
the two-writer transcript, owner tokens, the half-written/empty lock classification,
the crash-between-publish-and-temp-cleanup row, the forced-recovery row, the
changed-owner release/recovery transcript showing the successor record survives, the
unreadable-lock explicit-recovery transcript, the hard-link-unsupported
classification, lock-scope review and a source scan showing no hidden global mutable
state in the storage path. A review note records why the per-session lock and the root
allocation lock are the only serialization points.

**Existing precondition:** `SessionManager` and `Session` mutation require `&mut` and
use `BTreeMap`; `three_way_fork_is_isolated_and_diverges` and
`fork_current_does_not_mutate_the_source` cover in-process isolation. There is no
cross-process lock, lock publication, lease, owner token or key-allocation lock in the
tree.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `confinement.rs::noncanonical_key_components_are_rejected` | `cargo test -p chronofork-sessions --test confinement` |
| `confinement.rs::managed_symlinks_are_rejected_and_not_followed` | `cargo test -p chronofork-sessions --test confinement` |
| `confinement.rs::wrong_file_types_are_rejected` | `cargo test -p chronofork-sessions --test confinement` |
| `confinement.rs::names_never_become_path_components` | `cargo test -p chronofork-sessions --test confinement` |
| `confinement.rs::hostile_tree_cannot_escape_the_canonical_root` | `cargo test -p chronofork-sessions --test confinement` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A6-confinement.txt` with
the hostile tree layout, each attempted path, the observed typed error and a
before/after listing proving no file outside the root changed.

**Existing precondition:** there is no storage path code yet. The shared
`chronofork_core::codec::{Reader, Writer}` is bounds-checked but has no path API.
Windows reparse-point behavior remains unverified (ADR-0003 §11) and must not be
claimed by this plan.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `bounds.rs::oversized_checkpoint_is_rejected_before_full_read` | `cargo test -p chronofork-sessions --test bounds` |
| `bounds.rs::oversized_metadata_and_lock_records_are_typed` | `cargo test -p chronofork-sessions --test bounds` |
| `bounds.rs::file_that_grows_after_length_check_is_rejected` | `cargo test -p chronofork-sessions --test bounds` |
| `bounds.rs::list_entry_cap_is_enforced` | `cargo test -p chronofork-sessions --test bounds` |
| `bounds.rs::name_length_and_control_characters_are_rejected` | `cargo test -p chronofork-sessions --test bounds` |
| `bounds.rs::truncated_files_never_panic` | `cargo test -p chronofork-sessions --test bounds` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A7-bounds.txt` with the
lowered limit values, generated file sizes, observed errors and a code-review note
confirming length-before-allocation and bounded-reader use.

**Existing precondition:** `chronofork_core::codec` provides bounds-checked reading and
writing; `Snapshot::from_bytes` rejects short input and trailing bytes. There is no
file-length cap, growth check, list cap or name validation because no storage layer
exists.

**Status:** planned - not yet executed.

### M2-A8 — Determinism: the same checkpoint reproduces the same state hash after restart

**Passes only when** `checkpoint.cfsnap` is byte-for-byte the output of
`Snapshot::to_bytes()`, and restoring it after restart yields the same tick and state
hash as before the restart. Re-capturing after restore must reproduce the identical
canonical bytes and state hash. Changing a name, logical creation sequence or advisory
host timestamp must not change checkpoint bytes, the state hash or a replay result, and
host time must never order or validate snapshots. Two independent restarts and a
debug/release pair must produce identical reports. No wall-clock time, filesystem
modification time, process ID or entropy may enter the checkpoint or state hash.

| Planned test | Planned command |
|---|---|
| `determinism.rs::stored_checkpoint_bytes_equal_canonical_envelope` | `cargo test -p chronofork-sessions --test determinism` |
| `determinism.rs::restart_reproduces_tick_and_state_hash` | `cargo test -p chronofork-sessions --test determinism` |
| `determinism.rs::metadata_changes_do_not_change_checkpoint_bytes_or_state_hash` | `cargo test -p chronofork-sessions --test determinism` |
| `determinism.rs::two_restarts_produce_identical_reports` | `cargo test -p chronofork-sessions --test determinism` |
| `determinism.rs::restart_report_is_stable_within_a_mode` | `cargo test -p chronofork-sessions --test determinism` and `cargo test -p chronofork-sessions --release --test determinism` |
| `session_storage.rs::load_reports_the_same_hash_after_two_new_processes` | `cargo test -p chronofork-cli --test session_storage` |

Debug/release equality is compared **across the two recorded artifacts**, not inside one
test process: run the target in both modes and compare the reports field by field.

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A8-determinism.txt` with
the checkpoint SHA-256, canonical envelope SHA-256, tick, state hash, both debug and
release commands and both process reports.

**Existing precondition:** `Snapshot::to_bytes` is the canonical serializer and the
existing `serialized_checkpoints_round_trip_exactly` property test covers byte
round-trip in memory; `restore_and_replay_reproduce_the_branch_state` covers
restore-plus-replay in one process. Neither covers filesystem bytes, restart or
metadata variation.

**Status:** planned - not yet executed.

### M2-A9 — No mutable state shared across sessions, including persistence

**Passes only when** each rehydrated session owns its own mutable backend and
checkpoints; forking never shares mutable state with the source; publishing or deleting
one session or snapshot never changes another session's checkpoint bytes, state hash,
lineage or list ordering; independent readers see independent results; and storage adds
no hidden global mutable registry. The in-process ownership rules stay explicit:
mutation requires `&mut`, and shared access at a future service boundary needs its own
documented synchronization policy.

| Planned test | Planned command |
|---|---|
| `isolation.rs::rehydrated_sessions_do_not_share_mutable_state` | `cargo test -p chronofork-sessions --test isolation` |
| `isolation.rs::publishing_a_child_does_not_mutate_the_parent_checkpoint` | `cargo test -p chronofork-sessions --test isolation` |
| `isolation.rs::deleting_one_snapshot_leaves_other_sessions_unchanged` | `cargo test -p chronofork-sessions --test isolation` |
| `isolation.rs::independent_readers_are_side_effect_free` | `cargo test -p chronofork-sessions --test isolation` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A9-isolation.txt` with
before/after checkpoint SHA-256 and state hashes for every affected session, plus a
review note confirming no hidden global mutable state was introduced.

**Existing precondition:** `three_way_fork_is_isolated_and_diverges` and
`fork_current_does_not_mutate_the_source` in
`crates/chronofork-sessions/tests/acceptance.rs` cover in-process state and memory
isolation; `SessionManager` uses `&mut` and an ordered `BTreeMap`. No durable session
or storage-level isolation exists yet.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `crash_recovery.rs::staging_directory_is_reported_and_never_loaded` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::trash_tombstone_is_reported_and_never_resurrected` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::missing_or_corrupt_record_is_a_typed_anomaly` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::failed_publish_never_advances_visible_state` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::post_rename_parent_sync_failure_is_durability_unconfirmed` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::post_rename_failure_never_reports_success_or_retries_blindly` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::metadata_and_delete_post_rename_sync_failures_are_durability_unconfirmed` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::reconciliation_after_retried_parent_sync_confirms_visible_object` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::reconciliation_leaves_visible_object_unconfirmed` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::reconciliation_reports_object_lost_before_retry` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::stale_allocation_lock_recovery_is_explicit_and_owner_checked` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::allocation_lock_recovery_with_changed_owner_returns_not_lock_owner` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::allocation_lock_recovery_clears_lock_but_does_not_change_counter` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::unreadable_allocation_lock_recovery_is_corrupt_lock_and_removes_nothing` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::allocation_counter_update_failure_is_durability_unconfirmed` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::missing_short_oversized_or_unreadable_session_keys_is_corrupt_allocation_counter` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::persisted_counter_gap_is_never_reused` | `cargo test -p chronofork-sessions --test crash_recovery` |
| `crash_recovery.rs::key_and_sequence_reservations_are_never_reused` | `cargo test -p chronofork-sessions --test crash_recovery` |

**Planned evidence rows in A10-crash.txt:**

| Case | Row content |
|---|---|
| F10(a) retried parent sync confirms | object remains visible; retried parent sync succeeds; re-open/validation matches; outcome recorded as visible-confirmed |
| F10(b) still unconfirmed | object remains visible; parent sync still fails or is unconfirmed; no success is reported; outcome recorded as visible-unconfirmed |
| F10(c) lost before reconciliation | object disappeared before retry; re-open reports the loss; no resurrection or automatic repair |
| F12 corrupt/missing session-keys | missing, short, oversized and unreadable `state/session-keys` each return `CorruptAllocationCounter`; counter is not reset to one |
| F12 persisted-counter gap | gap is persisted; reserved key and every earlier gap key are never reused; next minted key is greater than every published key |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A10-crash.txt` with each
constructed tree, the observed diagnostic/error, the injected failure point (if any),
the post-rename `DurabilityUnconfirmed` transcript for publish/metadata/delete, the
F10 confirmed/unconfirmed/lost reconciliation rows, the allocation-lock recovery
transcript including owner/generation and the unchanged counter, the F12 corrupt
counter and persisted-gap rows, and the before/after object listing.

**Existing precondition:** none. The current tree has no publish sequence, staging
directory, tombstone, fsync, lock publication/recovery or storage path.

**Status:** planned - not yet executed.

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

| Planned test | Planned command |
|---|---|
| `retention.rs::list_is_deterministic_and_ordered` | `cargo test -p chronofork-sessions --test retention` |
| `retention.rs::invalid_entries_are_anomalies_not_silent_skips` | `cargo test -p chronofork-sessions --test retention` |
| `retention.rs::open_capture_close_and_restart_delete_nothing` | `cargo test -p chronofork-sessions --test retention` |
| `retention.rs::delete_is_explicit_target_only_and_non_cascading` | `cargo test -p chronofork-sessions --test retention` |
| `retention.rs::delete_session_with_snapshots_is_rejected` | `cargo test -p chronofork-sessions --test retention` |
| `retention.rs::existing_key_is_a_conflict_not_an_overwrite` | `cargo test -p chronofork-sessions --test retention` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/A11-retention.txt` with the
directory listing and hashes before/after each operation, anomaly diagnostics, and the
deterministic list output from two runs.

**Existing precondition:** none. `chronofork-sessions` has no filesystem I/O; the CLI's
only file writes are trace/state output paths and its only file read is the ROM path in
`crates/chronofork-cli/src/main.rs`, not session persistence. `FR-SNAPSHOT-006` is Planned in the
[functional requirements](../requirements/functional.md).

**Status:** planned - not yet executed.

## 6. Owner-approved minimal CLI surface (not implemented)

The current CLI dispatch in `crates/chronofork-cli/src/main.rs` supports `demo`,
`megadrive demo|trace` and `backend describe` only. It has no `session` subcommand and
no storage root concept. Running `chronofork session ...` today is an unknown-command
error. Do not add this surface to `chronofork help` or claim it until it is
implemented and accepted.

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

| Planned test | Planned command |
|---|---|
| `session_storage.rs::session_usage_requires_explicit_root` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::save_then_load_in_a_new_process_round_trips` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::save_name_is_visible_after_new_process` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::save_session_continue_and_save_appends_snapshot` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::list_is_deterministic_and_ordered` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::load_does_not_create_a_missing_root` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::unknown_session_exits_one_with_typed_diagnostic` | `cargo test -p chronofork-cli --test session_storage` |
| `session_storage.rs::delete_requires_an_explicit_target_and_never_cascades` | `cargo test -p chronofork-cli --test session_storage` |

**Evidence artifact:** `.research/m2-durable-sessions/fase-b/CLI-session.txt` with each
CLI invocation, exit code, stdout/stderr split, parsed keys and state hashes; the
continue-and-save record shows the same session key, one new snapshot key with a greater
`creation_sequence`, and unchanged bytes for the prior checkpoint.

**Status of the whole CLI surface:** owner-approved contract;
**not implemented and not yet executed**.

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
| `crates/chronofork-cli/tests/demo.rs` and `trace.rs` | CLI process-level test pattern, temp directories, deterministic output | A `session` subcommand or storage root |

Existing precondition commands, to be recorded with their real output when the plan is
executed (not run by this design task):

```sh
cargo test -p chronofork-sessions --test acceptance
cargo test -p chronofork-sessions --test snapshot_properties
cargo test -p chronofork-sessions --lib
cargo test -p chronofork-backend-api --lib
cargo test -p chronofork-cli
```

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

## 9. Execution procedure for Fase B

1. ADR-0003 was accepted on 2026-11-14; a separately approved Fase B plan fixes the
   package/path placement (default: the existing `chronofork-sessions` crate), final
   CLI spelling and test-only fault-injection decision. Any new crate needs a separate
   explicit owner decision because ADR-0003 §14 does not grant one.
2. Implement storage without a second snapshot serializer, reader or compatibility
   predicate; reuse `Snapshot::to_bytes`/`from_bytes`, `validate_envelope` and the
   shared `chronofork_core::codec`.
3. Add one test target at a time with its criterion ID in the test name or a comment;
   keep every criterion row `planned - not yet executed` until its command runs.
4. Run the target command, the existing precondition commands and the full repository
   gate (`./scripts/check.sh` or its documented equivalent); store raw evidence under
   `.research/m2-durable-sessions/fase-b/`.
5. Record the authoritative gate result in [quality gates](quality-gates.md) via its
   owner. Do not copy volatile test counts into this plan or the roadmap.
6. Ask the verifier to review the evidence. Only then mark the criterion and the
   milestone subitems; a self-review is not acceptance.

## 10. Out of scope and stated limitations

- No implementation, code change or execution is part of this Fase A document.
- No claim of Windows or network-filesystem durability; directory `fsync`/rename
  behavior there is unverified (ADR-0003 §4).
- No encryption, authentication, redaction, deduplication, compression or
  multi-tenant isolation; the writer lock is cooperative, not a security boundary.
- No automatic retention, garbage collection, background repair or silent migration in
  v1; retention is caller-driven and deletion is explicit.
- No multi-writer or distributed lock; no `unsafe`; no new dependency is approved.
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
  [quality gates](quality-gates.md); do not rewrite history or relabel a failure.
