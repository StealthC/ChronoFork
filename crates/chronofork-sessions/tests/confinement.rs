#![allow(clippy::expect_used)]

//! Root confinement, path traversal and file-type rejection tests.
//!
//! No caller-supplied string becomes a path component. Keys are canonical decimal
//! `u64` text, managed symlinks are rejected (including symlinked ancestors) and
//! a hostile tree can only produce typed errors.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, SessionError, SessionManager, SnapshotRepository, StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-confinement-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn save_first_snapshot(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    name: Option<&str>,
) -> (u64, u64) {
    let runtime = manager.create_session().expect("runtime session");
    manager
        .step(runtime, InputFrame::new(3))
        .expect("session step");
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
    let entry = repository
        .save_snapshot(session.session_key, snapshot, name)
        .expect("publish snapshot");
    (session.session_key, entry.snapshot_key)
}

#[test]
fn noncanonical_key_components_are_rejected() {
    let root = temp_root("noncanonical");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let sessions = root.join("v1/sessions");
    for name in [
        "session-0",
        "session-01",
        "session-+1",
        "session--1",
        "session-1x",
        "session-",
        "session-..",
        "session-18446744073709551616",
    ] {
        fs::create_dir(sessions.join(name)).expect("create hostile session directory");
    }
    let listing = repository.list_sessions().expect("list sessions");
    assert!(listing.entries.is_empty());
    for name in [
        "session-0",
        "session-01",
        "session-+1",
        "session--1",
        "session-1x",
        "session-",
        "session-..",
        "session-18446744073709551616",
    ] {
        assert!(
            listing.anomalies.iter().any(|anomaly| {
                anomaly.path == sessions.join(name)
                    && matches!(anomaly.kind, StorageAnomalyKind::InvalidKey)
            }),
            "{name} must be reported as a non-canonical key"
        );
    }

    let mut manager = new_manager();
    let (session_key, _) = save_first_snapshot(&repository, &mut manager, None);
    let snapshots = root.join(format!("v1/sessions/session-{session_key}/snapshots"));
    for name in ["snapshot-0", "snapshot-01", "snapshot-+1", "snapshot-"] {
        fs::create_dir(snapshots.join(name)).expect("create hostile snapshot directory");
    }
    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(listing.entries.len(), 1);
    for name in ["snapshot-0", "snapshot-01", "snapshot-+1", "snapshot-"] {
        assert!(
            listing.anomalies.iter().any(|anomaly| {
                anomaly.path == snapshots.join(name)
                    && matches!(anomaly.kind, StorageAnomalyKind::InvalidKey)
            }),
            "{name} must be reported as a non-canonical key"
        );
    }
}

#[cfg(unix)]
#[test]
fn managed_symlinks_are_rejected_and_not_followed() {
    use std::os::unix::fs::symlink;

    let root = temp_root("symlinks");
    let outside = root.parent().expect("root has a parent").join(format!(
        "chronofork-outside-{}-symlinks",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).expect("create outside directory");
    fs::write(outside.join("session.cfmeta"), b"outside").expect("write outside record");
    fs::write(outside.join("checkpoint.cfsnap"), b"outside").expect("write outside checkpoint");

    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let sessions = root.join("v1/sessions");
    symlink(&outside, sessions.join("session-1")).expect("symlink session directory");
    assert!(matches!(
        repository
            .load_snapshot(1, 1)
            .expect_err("symlinked session directory"),
        SessionError::SymlinkRejected(_)
    ));
    let listing = repository.list_sessions().expect("list sessions");
    assert!(listing.entries.is_empty());
    assert!(listing.anomalies.iter().any(|anomaly| matches!(
        &anomaly.kind,
        StorageAnomalyKind::CorruptObject(SessionError::SymlinkRejected(_))
    )));
    fs::remove_file(sessions.join("session-1")).expect("remove session symlink");

    let mut manager = new_manager();
    let (session_key, snapshot_key) = save_first_snapshot(&repository, &mut manager, None);
    let checkpoint = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{snapshot_key}/checkpoint.cfsnap"
    ));
    fs::remove_file(&checkpoint).expect("remove checkpoint");
    symlink(outside.join("checkpoint.cfsnap"), &checkpoint).expect("symlink checkpoint");
    assert!(matches!(
        repository
            .load_snapshot(session_key, snapshot_key)
            .expect_err("symlinked checkpoint"),
        SessionError::SymlinkRejected(_)
    ));

    let snapshots = root.join(format!("v1/sessions/session-{session_key}/snapshots"));
    fs::remove_dir_all(&snapshots).expect("remove snapshots directory");
    symlink(&outside, &snapshots).expect("symlink snapshots directory");
    assert!(matches!(
        repository
            .load_snapshot(session_key, snapshot_key)
            .expect_err("symlinked ancestor"),
        SessionError::SymlinkRejected(_)
    ));
    assert_eq!(
        fs::read(outside.join("session.cfmeta")).expect("read outside record"),
        b"outside",
        "storage must never write through a symlink"
    );
}

#[test]
fn wrong_file_types_are_rejected() {
    let root = temp_root("file-types");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let sessions = root.join("v1/sessions");
    fs::write(sessions.join("session-5"), b"not a directory").expect("write regular file");
    let listing = repository.list_sessions().expect("list sessions");
    assert!(listing.anomalies.iter().any(|anomaly| {
        anomaly.path == sessions.join("session-5")
            && matches!(
                &anomaly.kind,
                StorageAnomalyKind::CorruptObject(SessionError::WrongFileType(_))
            )
    }));

    let mut manager = new_manager();
    let (session_key, _) = save_first_snapshot(&repository, &mut manager, None);
    let snapshots = root.join(format!("v1/sessions/session-{session_key}/snapshots"));
    fs::write(snapshots.join("snapshot-5"), b"not a directory").expect("write regular file");
    let listing = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert!(listing.anomalies.iter().any(|anomaly| {
        anomaly.path == snapshots.join("snapshot-5")
            && matches!(
                &anomaly.kind,
                StorageAnomalyKind::CorruptObject(SessionError::WrongFileType(_))
            )
    }));

    let metadata = root.join(format!("v1/sessions/session-{session_key}/session.cfmeta"));
    fs::remove_file(&metadata).expect("remove metadata file");
    fs::create_dir(&metadata).expect("replace metadata with a directory");
    assert!(matches!(
        repository
            .session(session_key)
            .expect_err("wrong file type"),
        SessionError::WrongFileType(_)
    ));
}

#[test]
fn names_never_become_path_components() {
    let root = temp_root("names");
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
    let escape_name = format!("../chronofork-escape-{}", std::process::id());
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    let entry = repository
        .save_snapshot(session.session_key, snapshot, Some(&escape_name))
        .expect("names are labels, not paths");
    assert_eq!(entry.name.as_deref(), Some(escape_name.as_str()));
    let loaded = repository
        .load_snapshot(session.session_key, entry.snapshot_key)
        .expect("load named snapshot");
    assert_eq!(loaded.entry.name.as_deref(), Some(escape_name.as_str()));
    assert!(
        !root
            .parent()
            .expect("root has a parent")
            .join(format!("chronofork-escape-{}", std::process::id()))
            .exists(),
        "a name containing ../ must not create a file outside the root"
    );

    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("second checkpoint record");
    let slash = repository
        .save_snapshot(session.session_key, snapshot, Some("a/b"))
        .expect("a slash remains a label");
    assert_eq!(slash.name.as_deref(), Some("a/b"));

    let snapshot_id = manager.snapshot(runtime).expect("third checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("third checkpoint record");
    assert!(matches!(
        repository
            .save_snapshot(session.session_key, snapshot, Some("bad\nname"))
            .expect_err("control character"),
        SessionError::InvalidName(_)
    ));
    let snapshot_id = manager.snapshot(runtime).expect("fourth checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("fourth checkpoint record");
    let long_name = "n".repeat(257);
    assert!(matches!(
        repository
            .save_snapshot(session.session_key, snapshot, Some(&long_name))
            .expect_err("over-long name"),
        SessionError::InvalidName(_)
    ));
}

#[test]
fn hostile_tree_cannot_escape_the_canonical_root() {
    let root = temp_root("hostile");
    let outside = root
        .parent()
        .expect("root has a parent")
        .join(format!("chronofork-hostile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).expect("create outside directory");
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"untouched").expect("write sentinel");
    let outside_before: Vec<_> = fs::read_dir(&outside)
        .expect("read outside directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();

    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let sessions = root.join("v1/sessions");
    for name in ["session-..", "session-0", "session-1x"] {
        fs::create_dir(sessions.join(name)).expect("create hostile directory");
    }
    let listing = repository.list_sessions().expect("list hostile sessions");
    assert!(listing.entries.is_empty());
    assert!(!listing.anomalies.is_empty());
    assert!(matches!(
        repository
            .load_snapshot(1, 1)
            .expect_err("hostile key is not a session"),
        SessionError::UnknownStorageSession(1)
    ));
    assert_eq!(
        fs::read(&sentinel).expect("read sentinel"),
        b"untouched",
        "no operation may read or write outside the canonical root"
    );
    let outside_after: Vec<_> = fs::read_dir(&outside)
        .expect("read outside directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(outside_before, outside_after);
}
