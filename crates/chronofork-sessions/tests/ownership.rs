#![allow(clippy::expect_used)]

//! M2-A5 ownership: one writer per session, typed contention, owner-checked
//! release/recovery and no hidden global lock.
//!
//! These tests exercise the on-disk protocol directly through the public repository
//! API and the documented layout. Processes are simulated by independent repository
//! instances over the same root; the real cross-process CLI boundary is a later task.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, LockIdentity, SessionError, SessionManager, SnapshotEntry, SnapshotRepository,
    StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-ownership-{}-{name}",
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
            .step(session, InputFrame::new(7))
            .expect("session step succeeds");
    }
}

fn session_dir(root: &Path, session_key: u64) -> PathBuf {
    root.join(format!("v1/sessions/session-{session_key}"))
}

fn writer_lock_path(root: &Path, session_key: u64) -> PathBuf {
    session_dir(root, session_key).join("writer.lock")
}

fn allocation_lock_path(root: &Path) -> PathBuf {
    root.join("v1/state/allocation.lock")
}

fn publish_session(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    name: Option<&str>,
    steps: usize,
) -> (SessionId, u64, SnapshotEntry) {
    let runtime = manager.create_session().expect("runtime session");
    step(manager, runtime, steps);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            name,
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let entry = repository
        .save_snapshot(session.session_key, snapshot, name)
        .expect("publish snapshot");
    (runtime, session.session_key, entry)
}

fn codec_identity() -> LockIdentity {
    LockIdentity {
        owner_token: "0000000000000000".to_owned(),
        generation: 1,
    }
}

#[test]
fn second_writer_is_rejected_with_typed_contention() {
    let root = temp_root("second-writer");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key, _) = publish_session(&repository, &mut manager, Some("held"), 2);

    let first = SnapshotRepository::open(&root).expect("open first writer");
    let second = SnapshotRepository::open(&root).expect("open second writer");
    let held = first
        .acquire_writer_lock(session_key)
        .expect("first writer acquires");
    let error = second
        .acquire_writer_lock(session_key)
        .expect_err("a second writer must be rejected");
    assert!(
        matches!(
            &error,
            SessionError::WriterContended {
                session: Some(contended),
                owner: Some(owner),
            } if *contended == session_key && owner == held.identity()
        ),
        "expected typed contention naming the observed owner, got {error}"
    );
    assert!(
        error.to_string().contains(&held.identity().owner_token),
        "the contention diagnostic must name the observed owner token"
    );

    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let error = second
        .save_snapshot(session_key, snapshot, Some("contended"))
        .expect_err("a contended save must not wait or steal");
    assert!(
        matches!(
            &error,
            SessionError::WriterContended {
                session: Some(contended),
                ..
            } if *contended == session_key
        ),
        "expected writer contention from the mutating operation, got {error}"
    );

    first
        .release_writer_lock(&held)
        .expect("release writer lock");
    let retry = second
        .acquire_writer_lock(session_key)
        .expect("the lock is available after release");
    second.release_writer_lock(&retry).expect("release retry");
    assert!(
        !writer_lock_path(&root, session_key).exists(),
        "a released lock must not leave the canonical record behind"
    );
}

#[test]
fn half_written_or_empty_writer_lock_is_writer_contended_on_acquisition() {
    let root = temp_root("partial-lock");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);
    let path = writer_lock_path(&root, session_key);

    for bytes in [&b""[..], b"CFLOCK01", b"partial record"] {
        fs::write(&path, bytes).expect("write unreadable lock");
        let error = repository
            .acquire_writer_lock(session_key)
            .expect_err("an unreadable lock must never grant access");
        assert!(
            matches!(
                &error,
                SessionError::WriterContended {
                    session: Some(contended),
                    owner: None,
                } if *contended == session_key
            ),
            "an unreadable lock must be contention, never corruption, got {error}"
        );
        assert_eq!(
            fs::read(&path).expect("re-read unreadable lock"),
            bytes,
            "acquisition must remove nothing it does not own"
        );
        let recovery = repository
            .recover_writer_lock(session_key, &codec_identity(), true)
            .expect_err("explicit recovery of an unreadable lock");
        assert!(
            matches!(&recovery, SessionError::CorruptLock { path: found, .. } if found == &path),
            "explicit recovery must classify an unreadable record as CorruptLock, got {recovery}"
        );
        assert_eq!(
            fs::read(&path).expect("re-read lock after recovery"),
            bytes,
            "explicit recovery of an unreadable lock removes nothing"
        );
    }

    fs::remove_file(&path).expect("remove unreadable lock");
    let lock = repository
        .acquire_writer_lock(session_key)
        .expect("acquisition succeeds once the unreadable record is removed out of band");
    repository.release_writer_lock(&lock).expect("release");
}

#[test]
fn crash_between_lock_publish_and_temp_cleanup_keeps_lock_and_reports_diagnostic() {
    let root = temp_root("leftover-temp");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);
    let leftover = session_dir(&root, session_key).join("writer.lock.deadbeef.tmp");
    fs::write(&leftover, b"leftover private record").expect("write leftover private record");

    let lock = repository
        .acquire_writer_lock(session_key)
        .expect("a private leftover is never a lock");
    assert!(writer_lock_path(&root, session_key).exists());

    let listing = repository.list_sessions().expect("list sessions");
    assert!(
        listing.anomalies.iter().any(|anomaly| {
            anomaly.path == leftover
                && matches!(anomaly.kind, StorageAnomalyKind::LockProtocolRecord)
        }),
        "the leftover private record must be reported as a lock-protocol diagnostic"
    );
    assert!(
        listing
            .anomalies
            .iter()
            .all(|anomaly| anomaly.path != writer_lock_path(&root, session_key)),
        "the canonical lock must never be reported as a leftover"
    );

    repository
        .release_writer_lock(&lock)
        .expect("release locked");
    let again = repository
        .acquire_writer_lock(session_key)
        .expect("a leftover temp does not prevent acquisition");
    repository
        .recover_writer_lock(session_key, again.identity(), true)
        .expect("explicit recovery against the canonical record succeeds");
    assert!(
        !writer_lock_path(&root, session_key).exists(),
        "recovery removes the canonical record it owns"
    );
    assert!(
        leftover.exists(),
        "the private leftover is a diagnostic and is not reclaimed automatically"
    );
}

#[test]
fn forced_recovery_with_owner_generation_succeeds_and_changed_owner_returns_not_lock_owner() {
    let root = temp_root("forced-recovery");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);

    let lock = repository
        .acquire_writer_lock(session_key)
        .expect("acquire writer lock");
    let changed = repository
        .recover_writer_lock(session_key, &codec_identity(), true)
        .expect_err("a changed owner must not be recovered");
    assert!(
        matches!(&changed, SessionError::NotLockOwner { session: Some(found), .. } if *found == session_key),
        "expected NotLockOwner for a changed record, got {changed}"
    );
    assert!(
        writer_lock_path(&root, session_key).exists(),
        "a failed recovery must not delete the current record"
    );

    let unforced = repository
        .recover_writer_lock(session_key, lock.identity(), false)
        .expect_err("a non-expired lease requires force");
    assert!(
        matches!(
            &unforced,
            SessionError::WriterContended {
                owner: Some(owner),
                ..
            } if owner == lock.identity()
        ),
        "an unexpired observed lease must remain contention, got {unforced}"
    );

    repository
        .recover_writer_lock(session_key, lock.identity(), true)
        .expect("forced recovery with the matching owner/generation succeeds");
    assert!(!writer_lock_path(&root, session_key).exists());
    let successor = repository
        .acquire_writer_lock(session_key)
        .expect("a recovered lock is free");
    repository
        .release_writer_lock(&successor)
        .expect("release successor");
}

#[test]
fn release_or_recovery_with_changed_owner_keeps_successor_and_returns_not_lock_owner() {
    let root = temp_root("successor");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);

    let stale = repository
        .acquire_writer_lock(session_key)
        .expect("first owner acquires");
    repository
        .recover_writer_lock(session_key, stale.identity(), true)
        .expect("recover the first owner");
    let successor = repository
        .acquire_writer_lock(session_key)
        .expect("successor acquires after recovery");

    let release = repository
        .release_writer_lock(&stale)
        .expect_err("a stale handle must not release the successor");
    assert!(
        matches!(&release, SessionError::NotLockOwner { session: Some(found), .. } if *found == session_key),
        "expected NotLockOwner from the stale handle, got {release}"
    );
    let recovery = repository
        .recover_writer_lock(session_key, stale.identity(), false)
        .expect_err("stale recovery must not remove the successor");
    assert!(
        matches!(&recovery, SessionError::NotLockOwner { .. }),
        "expected NotLockOwner from stale recovery, got {recovery}"
    );

    let observed = repository
        .acquire_writer_lock(session_key)
        .expect_err("the successor must still own the lock");
    assert!(
        matches!(
            &observed,
            SessionError::WriterContended {
                owner: Some(owner),
                ..
            } if owner == successor.identity()
        ),
        "the successor record must be preserved, got {observed}"
    );

    repository
        .release_writer_lock(&successor)
        .expect("the real successor can release");
}

#[cfg(unix)]
#[test]
fn symlinked_writer_lock_is_rejected_and_never_followed() {
    use std::os::unix::fs::symlink;

    let root = temp_root("symlink-lock");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);
    let outside = root.join("outside-lock-record");
    fs::write(&outside, b"outside").expect("write outside record");
    let path = writer_lock_path(&root, session_key);
    symlink(&outside, &path).expect("symlink canonical lock");

    let error = repository
        .acquire_writer_lock(session_key)
        .expect_err("a symlinked lock is rejected");
    assert!(
        matches!(&error, SessionError::SymlinkRejected(found) if found == &path),
        "expected SymlinkRejected, got {error}"
    );
    let recovery = repository
        .recover_writer_lock(session_key, &codec_identity(), true)
        .expect_err("explicit recovery of a symlinked lock");
    assert!(
        matches!(&recovery, SessionError::CorruptLock { path: found, .. } if found == &path),
        "explicit recovery of a symlinked lock is CorruptLock, got {recovery}"
    );
    assert!(
        fs::symlink_metadata(&path)
            .expect("inspect lock path")
            .file_type()
            .is_symlink(),
        "storage must never remove or follow a symlinked lock"
    );
    assert_eq!(
        fs::read(&outside).expect("read outside record"),
        b"outside",
        "storage must never follow or truncate a symlinked lock"
    );
}

#[test]
fn expired_lease_is_not_auto_stolen() {
    let root = temp_root("expired-lease");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, _) = publish_session(&repository, &mut manager, None, 1);

    let lock = repository
        .acquire_writer_lock_with_lease(session_key, Duration::ZERO)
        .expect("acquire an immediately expired lease");
    assert!(
        lock.lease_expires_at_unix_ms().is_some(),
        "a host clock was available, so the advisory lease deadline is recorded"
    );
    let error = repository
        .acquire_writer_lock(session_key)
        .expect_err("an expired lease is never auto-stolen");
    assert!(
        matches!(&error, SessionError::WriterContended { owner: Some(owner), .. } if owner == lock.identity()),
        "expected contention on acquisition, got {error}"
    );

    repository
        .recover_writer_lock(session_key, lock.identity(), false)
        .expect("explicit recovery of an expired lease does not require force");
    let successor = repository
        .acquire_writer_lock(session_key)
        .expect("acquisition succeeds only after explicit recovery");
    repository.release_writer_lock(&successor).expect("release");
}

#[test]
fn key_allocation_lock_is_short_lived_and_root_scoped() {
    let root = temp_root("allocation");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, first_key, _) = publish_session(&repository, &mut manager, Some("first"), 1);
    assert!(
        !allocation_lock_path(&root).exists(),
        "the allocation lock is released after the key reservation"
    );

    let held = repository
        .acquire_writer_lock(first_key)
        .expect("hold a session writer lock");
    let second = repository
        .create_session(SessionId::from_raw(2), None, DurableLineage::root())
        .expect("session creation is not blocked by another session's writer lock");
    assert_eq!(second.session_key, first_key + 1);
    assert!(
        !allocation_lock_path(&root).exists(),
        "the root allocation lock is never held after session creation"
    );
    assert!(
        writer_lock_path(&root, first_key).exists(),
        "the held per-session lock remains while the root lock comes and goes"
    );

    let record = fs::read(writer_lock_path(&root, first_key)).expect("read held lock record");
    repository
        .release_writer_lock(&held)
        .expect("release writer lock");
    fs::write(allocation_lock_path(&root), record).expect("install a stale allocation lock");
    let error = repository
        .create_session(SessionId::from_raw(3), None, DurableLineage::root())
        .expect_err("a stale allocation lock blocks session-key minting");
    let SessionError::WriterContended {
        session: None,
        owner: Some(owner),
    } = &error
    else {
        unreachable!("expected allocation-lock contention with an observed owner, got {error}");
    };
    repository
        .recover_allocation_lock(owner, true)
        .expect("explicit root-lock recovery succeeds");
    let third = repository
        .create_session(SessionId::from_raw(3), None, DurableLineage::root())
        .expect("session creation resumes after recovery");
    assert_eq!(third.session_key, second.session_key + 1);
}

#[test]
fn distinct_sessions_write_concurrently_without_a_global_lock() {
    let root = temp_root("distinct");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_first_runtime, first_key, _) =
        publish_session(&repository, &mut manager, Some("first"), 1);
    let (second_runtime, second_key, _) =
        publish_session(&repository, &mut manager, Some("second"), 2);

    let first_lock = repository
        .acquire_writer_lock(first_key)
        .expect("first session writer acquires");
    assert!(writer_lock_path(&root, first_key).exists());

    step(&mut manager, second_runtime, 1);
    let snapshot_id = manager.snapshot(second_runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(second_runtime, snapshot_id)
        .expect("checkpoint record");
    let published = repository
        .save_snapshot(second_key, snapshot, Some("concurrent"))
        .expect("publish to the other session while the first writer holds its lock");
    assert!(published.snapshot_key > 1);

    let second_lock = repository
        .acquire_writer_lock(second_key)
        .expect("a different session is not blocked by the first writer");
    assert!(writer_lock_path(&root, second_key).exists());
    assert_eq!(
        repository
            .list_snapshots(first_key)
            .expect("list first")
            .entries
            .len(),
        1,
        "writing one session must not mutate another"
    );

    repository
        .release_writer_lock(&first_lock)
        .expect("release first");
    repository
        .release_writer_lock(&second_lock)
        .expect("release second");
}

#[test]
fn lock_free_reader_never_observes_a_torn_checkpoint() {
    let root = temp_root("lock-free-reader");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = publish_session(&repository, &mut manager, Some("reader"), 3);

    let lock = repository
        .acquire_writer_lock(session_key)
        .expect("hold the writer lock");
    let loaded = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("a reader takes no writer lock");
    assert_eq!(loaded.entry, entry);
    assert_eq!(
        repository
            .list_snapshots(session_key)
            .expect("lock-free list snapshots")
            .entries,
        vec![entry.clone()]
    );
    assert!(
        repository
            .list_sessions()
            .expect("lock-free list sessions")
            .entries
            .iter()
            .any(|session| session.session_key == session_key)
    );
    assert_eq!(
        loaded
            .snapshot
            .to_bytes()
            .expect("serialize loaded checkpoint"),
        fs::read(root.join(format!(
            "v1/sessions/session-{session_key}/snapshots/snapshot-{}/checkpoint.cfsnap",
            entry.snapshot_key
        )))
        .expect("read stored checkpoint"),
        "a lock-free reader sees the complete canonical bytes"
    );
    repository.release_writer_lock(&lock).expect("release");
}

#[test]
fn with_writer_lock_makes_read_modify_publish_linearizable() {
    let root = temp_root("with-writer-lock");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_runtime, session_key, _) = publish_session(&repository, &mut manager, Some("base"), 2);
    let second_handle = SnapshotRepository::open(&root).expect("open second handle");

    let entry = repository
        .with_writer_lock::<_, SessionError>(session_key, |repository, lock| {
            // A second process reaching acquisition while the workflow runs must fail
            // before it can read the same base.
            let contended = second_handle
                .acquire_writer_lock(session_key)
                .expect_err("the workflow holds the writer lock");
            assert!(
                matches!(&contended, SessionError::WriterContended { owner: Some(owner), .. } if owner == lock.identity()),
                "expected typed contention inside the workflow, got {contended}"
            );
            let latest = repository
                .latest_snapshot(session_key)?
                .expect("a committed base snapshot");
            let loaded = repository.load_snapshot(session_key, latest.snapshot_key)?;
            let runtime =
                manager.create_session_from_stored_snapshot(loaded.snapshot, session_key)?;
            step(&mut manager, runtime, 1);
            let snapshot_id = manager.snapshot(runtime)?;
            let record = manager.snapshot_record(runtime, snapshot_id)?;
            repository.save_snapshot_with_lock(lock, record, Some("continued"))
        })
        .expect("read-modify-publish workflow");
    assert_eq!(entry.creation_sequence, 2);
    assert_eq!(entry.name.as_deref(), Some("continued"));

    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[1].snapshot_key, entry.snapshot_key);
    assert!(
        !writer_lock_path(&root, session_key).exists(),
        "the workflow releases its lock"
    );
}

#[test]
fn save_snapshot_with_lock_rejects_released_and_foreign_handles() {
    let root = temp_root("with-lock-rejected");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key, _) = publish_session(&repository, &mut manager, Some("base"), 1);
    let lock = repository
        .acquire_writer_lock(session_key)
        .expect("hold writer lock");
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let published = repository
        .save_snapshot_with_lock(&lock, snapshot, Some("locked"))
        .expect("publish through the held lock");
    assert_eq!(published.creation_sequence, 2);
    repository.release_writer_lock(&lock).expect("release");

    let snapshot_id = manager.snapshot(runtime).expect("third checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let stale = repository
        .save_snapshot_with_lock(&lock, snapshot, None)
        .expect_err("a released handle must not publish");
    assert!(
        matches!(stale, SessionError::NotLockOwner { session: Some(found), .. } if found == session_key),
        "expected NotLockOwner for a released handle, got {stale}"
    );

    let other_root = temp_root("with-lock-foreign");
    let other = SnapshotRepository::initialize(&other_root).expect("initialize other root");
    let other_session = other
        .create_session(SessionId::from_raw(9), None, DurableLineage::root())
        .expect("other session");
    let other_lock = other
        .acquire_writer_lock(other_session.session_key)
        .expect("other lock");
    let foreign = repository
        .save_snapshot_with_lock(&other_lock, snapshot, None)
        .expect_err("a handle from another root must not publish");
    assert!(
        matches!(foreign, SessionError::NotLockOwner { session: Some(found), .. } if found == session_key),
        "expected NotLockOwner for a foreign-store handle, got {foreign}"
    );

    assert_eq!(
        repository
            .list_snapshots(session_key)
            .expect("list snapshots")
            .entries
            .len(),
        2,
        "rejected lock-aware publishes create nothing"
    );
}
