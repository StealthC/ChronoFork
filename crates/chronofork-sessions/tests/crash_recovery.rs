#![cfg(unix)]
#![allow(clippy::expect_used)]

//! M2-A10 crash/fault recovery: staging and tombstones are never committed data,
//! post-rename parent-sync failures are `DurabilityUnconfirmed`, reconciliation
//! distinguishes confirmed/unconfirmed/lost, and the allocation lock/counter fail
//! closed.
//!
//! Fault injection uses real filesystem permission changes: a directory without its
//! read bits still allows renames (write+execute) but makes the directory `sync_all`
//! open fail, which is exactly the post-rename parent-sync fault the ADR describes.
//! Crash-shaped trees (staging, tombstones, missing or corrupt records) are
//! constructed directly. Windows and network-filesystem durability remain unverified
//! limitations per ADR-0003 §4, hence the Unix gate.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, LockIdentity, ReconciliationOutcome, SessionError, SessionManager,
    SnapshotEntry, SnapshotRepository, StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-crash-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn step(manager: &mut SessionManager, session: SessionId, count: usize) {
    for _ in 0..count {
        manager
            .step(session, InputFrame::new(3))
            .expect("session step succeeds");
    }
}

fn session_dir(root: &Path, session_key: u64) -> PathBuf {
    root.join(format!("v1/sessions/session-{session_key}"))
}

fn snapshots_dir(root: &Path, session_key: u64) -> PathBuf {
    session_dir(root, session_key).join("snapshots")
}

fn snapshot_dir(root: &Path, session_key: u64, snapshot_key: u64) -> PathBuf {
    snapshots_dir(root, session_key).join(format!("snapshot-{snapshot_key}"))
}

fn session_keys_path(root: &Path) -> PathBuf {
    root.join("v1/state/session-keys")
}

fn allocation_lock_path(root: &Path) -> PathBuf {
    root.join("v1/state/allocation.lock")
}

/// Temporarily removes directory permission bits, restoring them on drop.
///
/// Removing read keeps renames and file opens by name working while making the
/// directory `sync_all` open fail, which injects the post-rename parent-sync fault.
/// Removing write makes staging creation fail, which injects a pre-commit I/O fault.
/// Neither needs a production test seam.
struct ModeRestriction {
    path: PathBuf,
    mode: u32,
}

impl ModeRestriction {
    fn without_read(path: &Path) -> Self {
        Self::apply_mask(path, !0o444)
    }

    fn without_write(path: &Path) -> Self {
        Self::apply_mask(path, !0o222)
    }

    fn apply_mask(path: &Path, mask: u32) -> Self {
        let mode = fs::metadata(path)
            .expect("read directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let mut permissions = fs::metadata(path)
            .expect("read directory metadata")
            .permissions();
        permissions.set_mode(mode & mask);
        fs::set_permissions(path, permissions).expect("restrict directory permissions");
        Self {
            path: path.to_path_buf(),
            mode,
        }
    }
}

impl Drop for ModeRestriction {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::metadata(&self.path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(self.mode);
            let _ = fs::set_permissions(&self.path, permissions);
        }
    }
}

fn create_durable_session(repository: &SnapshotRepository, manager: &mut SessionManager) -> u64 {
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("crash"),
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    session.session_key
}

fn save_snapshot(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    session_key: u64,
    runtime: SessionId,
) -> SnapshotEntry {
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    repository
        .save_snapshot(session_key, &snapshot, None)
        .expect("publish snapshot")
}

fn publish_session(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    steps: usize,
) -> (SessionId, u64, SnapshotEntry) {
    let runtime = manager.create_session().expect("runtime session");
    step(manager, runtime, steps);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("durable"),
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let entry = save_snapshot(repository, manager, session.session_key, runtime);
    (runtime, session.session_key, entry)
}

fn stale_allocation_record(
    root: &Path,
    repository: &SnapshotRepository,
    session_key: u64,
) -> LockIdentity {
    let expiry = repository
        .acquire_writer_lock_with_lease(session_key, std::time::Duration::ZERO)
        .expect("acquire an expired-leased writer lock");
    let identity = expiry.identity().clone();
    let bytes = fs::read(session_dir(root, session_key).join("writer.lock"))
        .expect("read writer lock record");
    repository
        .release_writer_lock(&expiry)
        .expect("release writer lock");
    fs::write(allocation_lock_path(root), bytes).expect("install stale allocation lock");
    identity
}

fn codec_identity() -> LockIdentity {
    LockIdentity {
        owner_token: "ffffffffffffffff".to_owned(),
        generation: 42,
    }
}

#[test]
fn staging_directory_is_reported_and_never_loaded() {
    let root = temp_root("staging");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, 1);

    let session_staging = root.join("v1/sessions/.staging-session-9-0");
    fs::create_dir(&session_staging).expect("create session staging");
    fs::write(session_staging.join("session.cfmeta"), b"partial").expect("write partial record");
    let snapshot_staging = snapshots_dir(&root, session_key).join(".staging-snapshot-9-0");
    fs::create_dir(&snapshot_staging).expect("create snapshot staging");
    fs::write(snapshot_staging.join("checkpoint.cfsnap"), b"partial")
        .expect("write partial checkpoint");

    let sessions = repository.list_sessions().expect("list sessions");
    assert_eq!(sessions.entries.len(), 1);
    assert!(sessions.anomalies.iter().any(|anomaly| {
        anomaly.path == session_staging
            && matches!(anomaly.kind, StorageAnomalyKind::IncompletePublish)
    }));
    let snapshots = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(snapshots.entries.len(), 1);
    assert!(snapshots.anomalies.iter().any(|anomaly| {
        anomaly.path == snapshot_staging
            && matches!(anomaly.kind, StorageAnomalyKind::IncompletePublish)
    }));
    assert!(
        repository.load_snapshot(9, 1).is_err(),
        "a staging directory is never loaded as a session or snapshot"
    );
}

#[test]
fn trash_tombstone_is_reported_and_never_resurrected() {
    let root = temp_root("trash");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = publish_session(&repository, &mut manager, 1);
    let directory = snapshot_dir(&root, session_key, entry.snapshot_key);
    let tombstone = snapshots_dir(&root, session_key).join(".trash-snapshot-1-0");
    fs::rename(&directory, &tombstone).expect("rename to tombstone");

    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert!(listing.entries.is_empty());
    assert!(listing.anomalies.iter().any(|anomaly| {
        anomaly.path == tombstone && matches!(anomaly.kind, StorageAnomalyKind::PendingDeletion)
    }));
    assert!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .is_err(),
        "a tombstone is never loaded"
    );

    drop(repository);
    let repository = SnapshotRepository::open(&root).expect("reopen root");
    assert!(
        repository
            .list_snapshots(session_key)
            .expect("list after reopen")
            .entries
            .is_empty(),
        "reopening must not resurrect a tombstone"
    );
    assert!(
        !snapshot_dir(&root, session_key, entry.snapshot_key).exists(),
        "no operation recreates a deleted snapshot directory"
    );
}

#[test]
fn missing_or_corrupt_record_is_a_typed_anomaly() {
    let root = temp_root("anomaly");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = publish_session(&repository, &mut manager, 1);
    let directory = snapshot_dir(&root, session_key, entry.snapshot_key);
    let checkpoint_path = directory.join("checkpoint.cfsnap");
    let metadata_path = directory.join("snapshot.cfmeta");
    let checkpoint = fs::read(&checkpoint_path).expect("read checkpoint");
    let metadata = fs::read(&metadata_path).expect("read metadata");

    fs::remove_file(&checkpoint_path).expect("remove checkpoint");
    assert!(
        matches!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .expect_err("missing checkpoint"),
            SessionError::MissingRecord(_)
        ),
        "a missing checkpoint is a typed record error, not a silent skip"
    );
    fs::write(&checkpoint_path, &checkpoint).expect("restore checkpoint");

    fs::write(&metadata_path, b"corrupt").expect("corrupt metadata");
    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert!(listing.anomalies.iter().any(|anomaly| {
        anomaly.path == directory
            && matches!(
                &anomaly.kind,
                StorageAnomalyKind::CorruptObject(SessionError::CorruptMetadata(_))
            )
    }));
    assert_eq!(
        fs::read(&metadata_path).expect("re-read corrupt metadata"),
        b"corrupt",
        "listing must never repair an anomalous record"
    );
    fs::write(&metadata_path, &metadata).expect("restore metadata");
    assert!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .is_ok(),
        "the restored record is valid again"
    );
}

#[test]
fn failed_publish_never_advances_visible_state() {
    let root = temp_root("failed-publish");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("durable session");
    let session_key = session.session_key;
    let snapshots = snapshots_dir(&root, session_key);

    let restricted = ModeRestriction::without_write(&snapshots);
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let error = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect_err("a publish that cannot create staging fails");
    drop(restricted);
    assert!(
        matches!(
            &error,
            SessionError::Io {
                operation: "create staging directory",
                ..
            }
        ),
        "a pre-commit failure is a typed I/O error, got {error}"
    );
    assert!(
        !snapshot_dir(&root, session_key, 1).exists(),
        "a failed publish must leave no visible snapshot"
    );
    assert!(
        repository
            .list_snapshots(session_key)
            .expect("list snapshots")
            .entries
            .is_empty()
    );
    assert_eq!(
        repository
            .session(session_key)
            .expect("session record")
            .next_snapshot_key,
        2,
        "the reservation advanced, so the failed key is a permanent gap"
    );

    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let retried = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect("a new publish uses the next reserved key");
    assert_eq!(retried.snapshot_key, 2);
    assert!(
        !snapshot_dir(&root, session_key, 1).exists(),
        "the failed key is never reused"
    );
}

#[test]
fn post_rename_parent_sync_failure_is_durability_unconfirmed() {
    let root = temp_root("post-rename");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("durable session");
    let session_key = session.session_key;
    let snapshots = snapshots_dir(&root, session_key);

    let restricted = ModeRestriction::without_read(&snapshots);
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let error = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect_err("a failed parent sync is never reported as success");
    drop(restricted);
    assert!(
        matches!(
            &error,
            SessionError::DurabilityUnconfirmed {
                operation: "publish snapshot",
                path,
            } if path == &snapshots
        ),
        "expected publish DurabilityUnconfirmed, got {error}"
    );
    assert!(
        !session_dir(&root, session_key).join("writer.lock").exists(),
        "the writer lock is released before a DurabilityUnconfirmed error is returned"
    );

    let visible = repository
        .load_snapshot(session_key, 1)
        .expect("the visible object re-validates");
    assert_eq!(
        visible.snapshot.to_bytes().expect("serialize"),
        fs::read(snapshot_dir(&root, session_key, 1).join("checkpoint.cfsnap"))
            .expect("read checkpoint"),
        "the visible object is the complete canonical checkpoint"
    );
}

#[test]
fn post_rename_failure_never_reports_success_or_retries_blindly() {
    let root = temp_root("no-blind-retry");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("durable session");
    let session_key = session.session_key;
    let snapshots = snapshots_dir(&root, session_key);
    let first_id = manager.snapshot(runtime).expect("first checkpoint");
    let first = manager
        .snapshot_record(runtime, first_id)
        .expect("first checkpoint record");

    let restricted = ModeRestriction::without_read(&snapshots);
    let error = repository
        .save_snapshot(session_key, &first, None)
        .expect_err("unconfirmed publish");
    drop(restricted);
    assert!(matches!(error, SessionError::DurabilityUnconfirmed { .. }));
    assert!(snapshot_dir(&root, session_key, 1).exists());

    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("second checkpoint record");
    let second = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect("a caller-driven retry publishes a new reserved key");
    assert_eq!(second.snapshot_key, 2);
    assert!(
        snapshot_dir(&root, session_key, 1).exists(),
        "the unconfirmed object is not overwritten by a blind retry"
    );
    assert_eq!(
        repository
            .reconcile_snapshot(session_key, 1)
            .expect("reconcile the unconfirmed publish"),
        ReconciliationOutcome::VisibleConfirmed
    );
}

#[test]
fn metadata_and_delete_post_rename_sync_failures_are_durability_unconfirmed() {
    let root = temp_root("metadata-delete");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = publish_session(&repository, &mut manager, 1);
    let directory = snapshot_dir(&root, session_key, entry.snapshot_key);

    let restricted = ModeRestriction::without_read(&directory);
    let error = repository
        .rename_snapshot(session_key, entry.snapshot_key, Some("renamed"))
        .expect_err("metadata post-rename sync failure");
    drop(restricted);
    assert!(
        matches!(
            &error,
            SessionError::DurabilityUnconfirmed {
                operation: "replace snapshot metadata",
                ..
            }
        ),
        "expected metadata DurabilityUnconfirmed, got {error}"
    );
    assert_eq!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect("load renamed metadata")
            .entry
            .name
            .as_deref(),
        Some("renamed")
    );
    assert!(
        !session_dir(&root, session_key).join("writer.lock").exists(),
        "the writer lock is released before the metadata error is returned"
    );

    let snapshots = snapshots_dir(&root, session_key);
    let restricted = ModeRestriction::without_read(&snapshots);
    let error = repository
        .delete_snapshot(session_key, entry.snapshot_key)
        .expect_err("delete post-rename sync failure");
    drop(restricted);
    assert!(
        matches!(
            &error,
            SessionError::DurabilityUnconfirmed {
                operation: "delete snapshot",
                ..
            }
        ),
        "expected delete DurabilityUnconfirmed, got {error}"
    );
    let listing = repository
        .list_snapshots(session_key)
        .expect("list after unconfirmed delete");
    assert!(listing.entries.is_empty());
    assert!(
        listing
            .anomalies
            .iter()
            .any(|anomaly| matches!(anomaly.kind, StorageAnomalyKind::PendingDeletion)),
        "the delete tombstone is reported and never resurrected"
    );
    assert!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .is_err()
    );
}

#[test]
fn reconciliation_after_retried_parent_sync_confirms_visible_object() {
    let (root, repository, session_key) = unconfirmed_publish("reconcile-confirmed");
    let outcome = repository
        .reconcile_snapshot(session_key, 1)
        .expect("reconcile");
    assert_eq!(outcome, ReconciliationOutcome::VisibleConfirmed);
    assert!(
        repository.load_snapshot(session_key, 1).is_ok(),
        "a confirmed object re-validates"
    );
    assert!(!allocation_lock_path(&root).exists());

    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].snapshot_key, 1);
}

#[test]
fn reconciliation_leaves_visible_object_unconfirmed() {
    let (root, repository, session_key) = unconfirmed_publish("reconcile-unconfirmed");
    let snapshots = snapshots_dir(&root, session_key);
    let restricted = ModeRestriction::without_read(&snapshots);
    let outcome = repository
        .reconcile_snapshot(session_key, 1)
        .expect("reconcile while the parent sync still fails");
    assert_eq!(outcome, ReconciliationOutcome::VisibleUnconfirmed);
    drop(restricted);
    assert!(
        repository
            .list_snapshots(session_key)
            .expect("list visible object")
            .entries
            .iter()
            .any(|entry| entry.snapshot_key == 1),
        "the object remains visible but no success was reported"
    );
}

#[test]
fn reconciliation_reports_object_lost_before_retry() {
    let (root, repository, session_key) = unconfirmed_publish("reconcile-lost");
    fs::remove_dir_all(snapshot_dir(&root, session_key, 1)).expect("simulate a lost object");
    let outcome = repository
        .reconcile_snapshot(session_key, 1)
        .expect("reconcile a lost object");
    assert_eq!(outcome, ReconciliationOutcome::Lost);
    assert!(
        !snapshot_dir(&root, session_key, 1).exists(),
        "reconciliation never resurrects or repairs a lost object"
    );
}

fn unconfirmed_publish(name: &str) -> (PathBuf, SnapshotRepository, u64) {
    let root = temp_root(name);
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("durable session");
    let session_key = session.session_key;
    let snapshots = snapshots_dir(&root, session_key);
    let restricted = ModeRestriction::without_read(&snapshots);
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let error = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect_err("unconfirmed publish");
    drop(restricted);
    assert!(matches!(&error, SessionError::DurabilityUnconfirmed { .. }));
    (root, repository, session_key)
}

#[test]
fn stale_allocation_lock_recovery_is_explicit_and_owner_checked() {
    let root = temp_root("allocation-stale");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let session_key = create_durable_session(&repository, &mut manager);
    let identity = stale_allocation_record(&root, &repository, session_key);

    let error = repository
        .create_session(SessionId::from_raw(2), None, DurableLineage::root())
        .expect_err("a stale allocation lock blocks key minting");
    assert!(
        matches!(
            &error,
            SessionError::WriterContended {
                session: None,
                owner: Some(owner),
            } if owner == &identity
        ),
        "expected allocation-lock contention, got {error}"
    );

    let changed = repository
        .recover_allocation_lock(&codec_identity(), true)
        .expect_err("a changed owner must not be recovered");
    assert!(matches!(
        changed,
        SessionError::NotLockOwner { session: None, .. }
    ));
    assert!(allocation_lock_path(&root).exists());

    repository
        .recover_allocation_lock(&identity, false)
        .expect("an expired observed lease recovers without force");
    assert!(!allocation_lock_path(&root).exists());
    let next = repository
        .create_session(SessionId::from_raw(2), None, DurableLineage::root())
        .expect("key minting resumes");
    assert_eq!(next.session_key, 2);
}

#[test]
fn allocation_lock_recovery_with_changed_owner_returns_not_lock_owner() {
    let root = temp_root("allocation-changed");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let session_key = create_durable_session(&repository, &mut manager);
    let identity = stale_allocation_record(&root, &repository, session_key);
    let before = fs::read(allocation_lock_path(&root)).expect("read stale lock");

    let error = repository
        .recover_allocation_lock(&codec_identity(), true)
        .expect_err("a changed record is never removed");
    assert!(matches!(
        error,
        SessionError::NotLockOwner { session: None, .. }
    ));
    assert_eq!(
        fs::read(allocation_lock_path(&root)).expect("re-read stale lock"),
        before,
        "a non-owner recovery changes nothing"
    );
    repository
        .recover_allocation_lock(&identity, true)
        .expect("the observed owner can recover");
}

#[test]
fn allocation_lock_recovery_clears_lock_but_does_not_change_counter() {
    let root = temp_root("allocation-counter");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let session_key = create_durable_session(&repository, &mut manager);
    let identity = stale_allocation_record(&root, &repository, session_key);
    let before = fs::read(session_keys_path(&root)).expect("read counter");

    repository
        .recover_allocation_lock(&identity, true)
        .expect("recover allocation lock");
    assert!(!allocation_lock_path(&root).exists());
    assert_eq!(
        fs::read(session_keys_path(&root)).expect("re-read counter"),
        before,
        "allocation-lock recovery clears the lock only"
    );
}

#[test]
fn unreadable_allocation_lock_recovery_is_corrupt_lock_and_removes_nothing() {
    let root = temp_root("allocation-unreadable");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let path = allocation_lock_path(&root);
    fs::write(&path, b"").expect("write empty allocation lock");

    let error = repository
        .create_session(SessionId::from_raw(1), None, DurableLineage::root())
        .expect_err("an unreadable allocation lock is contention");
    assert!(matches!(
        error,
        SessionError::WriterContended {
            session: None,
            owner: None,
        }
    ));
    let recovery = repository
        .recover_allocation_lock(&codec_identity(), true)
        .expect_err("explicit recovery of an unreadable lock");
    assert!(
        matches!(&recovery, SessionError::CorruptLock { path: found, .. } if found == &path),
        "expected CorruptLock, got {recovery}"
    );
    assert_eq!(
        fs::read(&path).expect("re-read lock"),
        b"",
        "explicit recovery of an unreadable lock removes nothing"
    );
}

#[test]
fn allocation_counter_update_failure_is_durability_unconfirmed() {
    let root = temp_root("counter-sync");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let state = root.join("v1/state");
    let restricted = ModeRestriction::without_read(&state);
    let error = repository
        .create_session(SessionId::from_raw(1), None, DurableLineage::root())
        .expect_err("an unconfirmed counter update publishes nothing");
    drop(restricted);
    assert!(
        matches!(
            &error,
            SessionError::DurabilityUnconfirmed {
                operation: "replace session-key counter",
                ..
            }
        ),
        "expected allocation DurabilityUnconfirmed, got {error}"
    );
    assert!(
        !session_dir(&root, 1).exists(),
        "no session is published when the counter update is unconfirmed"
    );
    assert_eq!(
        fs::read(session_keys_path(&root)).expect("read counter"),
        2_u64.to_le_bytes(),
        "the unconfirmed update is visible but never reported as durable"
    );

    let session = repository
        .create_session(SessionId::from_raw(1), None, DurableLineage::root())
        .expect("a later reservation continues past the gap");
    assert_eq!(session.session_key, 2);
    assert!(
        !session_dir(&root, 1).exists(),
        "the unconfirmed key is a gap and is never reused"
    );
}

#[test]
fn missing_short_oversized_or_unreadable_session_keys_is_corrupt_allocation_counter() {
    for (name, mutation) in [
        ("missing", 0_u8),
        ("short", 1),
        ("oversized", 2),
        ("unreadable", 3),
    ] {
        let root = temp_root(&format!("counter-{name}"));
        let repository = SnapshotRepository::initialize(&root).expect("initialize root");
        let path = session_keys_path(&root);
        match mutation {
            0 => fs::remove_file(&path).expect("remove counter"),
            1 => fs::write(&path, 4_u32.to_le_bytes()).expect("write short counter"),
            2 => fs::write(&path, [0_u8; 16]).expect("write oversized counter"),
            _ => {
                let mut permissions = fs::metadata(&path).expect("counter metadata").permissions();
                permissions.set_mode(0o000);
                fs::set_permissions(&path, permissions).expect("make counter unreadable");
            }
        }
        let error = repository
            .create_session(SessionId::from_raw(1), None, DurableLineage::root())
            .expect_err("a corrupt counter fails closed");
        assert!(
            matches!(error, SessionError::CorruptAllocationCounter(_)),
            "{name}: expected CorruptAllocationCounter, got {error}"
        );
        if mutation == 3 {
            let mut permissions = fs::metadata(&path).expect("counter metadata").permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(&path, permissions).expect("restore counter permissions");
            assert_eq!(
                fs::read(&path).expect("read counter"),
                1_u64.to_le_bytes(),
                "the counter is never reset to one automatically"
            );
        } else {
            assert!(
                !repository
                    .list_sessions()
                    .expect("list sessions")
                    .entries
                    .iter()
                    .any(|entry| entry.session_key == 1),
                "{name}: no session is published with a corrupt counter"
            );
        }
    }
}

#[test]
fn persisted_counter_gap_is_never_reused() {
    let root = temp_root("counter-gap");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    fs::write(session_keys_path(&root), 5_u64.to_le_bytes()).expect("persist a gap");

    let first = repository
        .create_session(SessionId::from_raw(1), None, DurableLineage::root())
        .expect("first reservation");
    let second = repository
        .create_session(SessionId::from_raw(2), None, DurableLineage::root())
        .expect("second reservation");
    let third = repository
        .create_session(SessionId::from_raw(3), None, DurableLineage::root())
        .expect("third reservation");
    assert_eq!(
        (first.session_key, second.session_key, third.session_key),
        (5, 6, 7)
    );
    for earlier in 1..=4 {
        assert!(
            !session_dir(&root, earlier).exists(),
            "gap key {earlier} is never reused"
        );
    }
    assert_eq!(
        fs::read(session_keys_path(&root)).expect("read counter"),
        8_u64.to_le_bytes(),
        "the next minted key is greater than every published key"
    );
}

#[test]
fn key_and_sequence_reservations_are_never_reused() {
    let root = temp_root("reservations");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key, first) = publish_session(&repository, &mut manager, 1);
    assert_eq!(first.snapshot_key, 1);

    let snapshots = snapshots_dir(&root, session_key);
    let restricted = ModeRestriction::without_read(&snapshots);
    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let error = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect_err("unconfirmed publish");
    drop(restricted);
    assert!(matches!(error, SessionError::DurabilityUnconfirmed { .. }));
    assert!(
        snapshot_dir(&root, session_key, 2).exists(),
        "the reserved key is visible but unconfirmed"
    );

    repository
        .delete_snapshot(session_key, 2)
        .expect("delete the unconfirmed object");
    let snapshot_id = manager.snapshot(runtime).expect("third checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let third = repository
        .save_snapshot(session_key, &snapshot, None)
        .expect("publish after the reserved object is lost");
    assert_eq!(third.snapshot_key, 3);
    assert_eq!(third.creation_sequence, 3);
    assert!(
        !snapshot_dir(&root, session_key, 2).exists(),
        "a reserved key and sequence are never reused"
    );
    assert!(snapshot_dir(&root, session_key, 1).exists());
    assert!(snapshot_dir(&root, session_key, 3).exists());
}
