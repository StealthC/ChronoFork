#![allow(clippy::expect_used)]

//! Storage bounds: size limits, bounded reads, list caps and name validation.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, SessionError, SessionManager, SnapshotEntry, SnapshotRepository, StorageLimits,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-bounds-{}-{name}",
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
            .step(session, InputFrame::new(2))
            .expect("session step succeeds");
    }
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
        .save_snapshot(session_key, snapshot, None)
        .expect("publish snapshot")
}

fn setup(name: &str) -> (PathBuf, SnapshotRepository, u64, SnapshotEntry) {
    let root = temp_root(name);
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 3);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let entry = save_snapshot(&repository, &mut manager, session.session_key, runtime);
    (root, repository, session.session_key, entry)
}

#[test]
fn oversized_checkpoint_is_rejected_before_full_read() {
    let (root, _repository, session_key, entry) = setup("checkpoint");
    let limited = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_checkpoint_bytes: 16,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a lowered checkpoint limit");
    assert!(matches!(
        limited
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("oversized checkpoint"),
        SessionError::FileTooLarge { limit: 16, .. }
    ));
}

#[test]
fn oversized_metadata_is_rejected_before_full_read() {
    let (root, _repository, session_key, entry) = setup("metadata");
    let session_length =
        fs::metadata(root.join(format!("v1/sessions/session-{session_key}/session.cfmeta")))
            .expect("session metadata metadata")
            .len();
    let snapshot_length = fs::metadata(root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{}/snapshot.cfmeta",
        entry.snapshot_key
    )))
    .expect("snapshot metadata metadata")
    .len();
    assert!(session_length < snapshot_length);
    let limited = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_metadata_bytes: session_length + 1,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a lowered metadata limit");
    assert!(matches!(
        limited
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("oversized snapshot metadata"),
        SessionError::FileTooLarge { .. }
    ));

    let too_small = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_metadata_bytes: 8,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a tiny metadata limit");
    assert!(matches!(
        too_small
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("oversized session metadata"),
        SessionError::FileTooLarge { limit: 8, .. }
    ));
}

#[test]
fn oversized_metadata_is_rejected_at_publish_without_consuming_a_key() {
    let root = temp_root("metadata-publish");
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
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let session_length = fs::metadata(root.join(format!(
        "v1/sessions/session-{}/session.cfmeta",
        session.session_key
    )))
    .expect("session metadata metadata")
    .len();
    let limited = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_metadata_bytes: session_length + 1,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a lowered metadata limit");
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    assert!(matches!(
        limited
            .save_snapshot(session.session_key, snapshot, None)
            .expect_err("oversized snapshot metadata"),
        SessionError::FileTooLarge { .. }
    ));
    assert_eq!(
        limited
            .session(session.session_key)
            .expect("session record")
            .next_snapshot_key,
        1,
        "a rejected metadata record must not reserve a snapshot key"
    );

    let too_small = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_metadata_bytes: 8,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a tiny metadata limit");
    assert!(matches!(
        too_small
            .create_session(SessionId::from_raw(1), None, DurableLineage::root())
            .expect_err("oversized session metadata"),
        SessionError::FileTooLarge { limit: 8, .. }
    ));
}

#[test]
fn checkpoint_growth_after_the_length_check_is_rejected() {
    let (root, repository, session_key, entry) = setup("growth");
    let checkpoint = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{}/checkpoint.cfsnap",
        entry.snapshot_key
    ));
    let mut bytes = fs::read(&checkpoint).expect("read checkpoint");
    bytes.push(0);
    fs::write(&checkpoint, bytes).expect("append to checkpoint");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("growth inside the size limit still breaks the binding"),
        SessionError::MetadataMismatch(_)
    ));
}

#[test]
fn list_entry_cap_is_enforced() {
    let root = temp_root("list-cap");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    for _ in 0..3 {
        repository
            .create_session(SessionId::from_raw(1), None, DurableLineage::root())
            .expect("durable session");
    }
    let limited = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_list_entries: 2,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a lowered list cap");
    assert_eq!(
        limited.list_sessions().expect_err("list cap"),
        SessionError::ListLimitExceeded { limit: 2 }
    );

    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("snapshot session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    for _ in 0..3 {
        step(&mut manager, runtime, 1);
        save_snapshot(&repository, &mut manager, session.session_key, runtime);
    }
    let limited = SnapshotRepository::open_with_limits(
        &root,
        StorageLimits {
            max_list_entries: 2,
            ..StorageLimits::default()
        },
    )
    .expect("reopen with a lowered list cap");
    assert_eq!(
        limited
            .list_snapshots(session.session_key)
            .expect_err("snapshot list cap"),
        SessionError::ListLimitExceeded { limit: 2 }
    );
}

#[test]
fn name_length_and_control_characters_are_rejected() {
    let root = temp_root("names");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let long_name = "n".repeat(257);
    assert!(matches!(
        repository
            .create_session(
                SessionId::from_raw(1),
                Some(&long_name),
                DurableLineage::root()
            )
            .expect_err("over-long session name"),
        SessionError::InvalidName(_)
    ));
    for bad in ["bad\nname", "bad\u{0}name", "bad\u{7}name"] {
        assert!(matches!(
            repository
                .create_session(SessionId::from_raw(1), Some(bad), DurableLineage::root())
                .expect_err("control character in session name"),
            SessionError::InvalidName(_)
        ));
    }
    let exact = "n".repeat(256);
    let session = repository
        .create_session(SessionId::from_raw(1), Some(&exact), DurableLineage::root())
        .expect("a 256-byte name is allowed");
    assert_eq!(session.name.as_deref(), Some(exact.as_str()));

    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
            DurableLineage::root(),
        )
        .expect("snapshot session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    assert!(matches!(
        repository
            .save_snapshot(session.session_key, snapshot, Some(&long_name))
            .expect_err("over-long snapshot name"),
        SessionError::InvalidName(_)
    ));
    assert_eq!(
        repository
            .session(session.session_key)
            .expect("session record")
            .next_snapshot_key,
        1,
        "a rejected name must not consume a snapshot key"
    );
}

#[test]
fn truncated_files_never_panic() {
    let (root, repository, session_key, entry) = setup("truncated");
    let directory = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{}",
        entry.snapshot_key
    ));
    let checkpoint_path = directory.join("checkpoint.cfsnap");
    let metadata_path = directory.join("snapshot.cfmeta");
    let checkpoint = fs::read(&checkpoint_path).expect("read checkpoint");
    let metadata = fs::read(&metadata_path).expect("read metadata");

    for length in 0..checkpoint.len().min(48) {
        fs::write(&checkpoint_path, &checkpoint[..length]).expect("write truncated checkpoint");
        assert!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .is_err()
        );
        assert!(repository.list_snapshots(session_key).is_ok());
    }
    fs::write(&checkpoint_path, &checkpoint).expect("restore checkpoint");

    for length in 0..metadata.len().min(48) {
        fs::write(&metadata_path, &metadata[..length]).expect("write truncated metadata");
        assert!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .is_err()
        );
        assert!(repository.list_snapshots(session_key).is_ok());
    }
}

#[test]
fn oversized_checkpoint_is_rejected_at_save_without_consuming_a_key() {
    let root = temp_root("save-limit");
    let repository = SnapshotRepository::initialize_with_limits(
        &root,
        StorageLimits {
            max_checkpoint_bytes: 16,
            ..StorageLimits::default()
        },
    )
    .expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            None,
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
    assert!(matches!(
        repository
            .save_snapshot(session.session_key, snapshot, None)
            .expect_err("oversized checkpoint"),
        SessionError::FileTooLarge { limit: 16, .. }
    ));
    assert_eq!(
        repository
            .session(session.session_key)
            .expect("session record")
            .next_snapshot_key,
        1,
        "a rejected checkpoint must not reserve a snapshot key"
    );
}
