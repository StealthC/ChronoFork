#![allow(clippy::expect_used)]

//! Explicit retention: deterministic listing, typed anomalies, explicit deletion
//! and no automatic garbage collection.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, SessionError, SessionManager, SnapshotEntry, SnapshotRepository,
    StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-retention-{}-{name}",
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
            .step(session, InputFrame::new(4))
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

fn create_session(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    name: Option<&str>,
    lineage: DurableLineage,
) -> (SessionId, u64) {
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(SessionId::from_raw(runtime.raw()), name, lineage)
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    (runtime, session.session_key)
}

fn tree(root: &Path) -> Vec<(String, u64)> {
    fn walk(base: &Path, directory: &Path, output: &mut Vec<(String, u64)>) {
        let mut paths: Vec<PathBuf> = fs::read_dir(directory)
            .expect("read directory")
            .map(|entry| entry.expect("directory entry").path())
            .collect();
        paths.sort();
        for path in paths {
            let relative = path
                .strip_prefix(base)
                .expect("path is beneath the root")
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                output.push((relative, 0));
                walk(base, &path, output);
            } else {
                output.push((relative, fs::metadata(&path).expect("file metadata").len()));
            }
        }
    }
    let mut output = Vec::new();
    walk(root, root, &mut output);
    output
}

#[test]
fn list_is_deterministic_and_ordered() {
    let root = temp_root("ordered");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, first_key) = create_session(
        &repository,
        &mut manager,
        Some("first"),
        DurableLineage::root(),
    );
    let mut first_snapshots = Vec::new();
    for _ in 0..3 {
        step(&mut manager, runtime, 1);
        first_snapshots.push(save_snapshot(&repository, &mut manager, first_key, runtime));
    }
    let (second_runtime, second_key) = create_session(
        &repository,
        &mut manager,
        Some("second"),
        DurableLineage::root(),
    );
    step(&mut manager, second_runtime, 1);
    save_snapshot(&repository, &mut manager, second_key, second_runtime);

    let sessions = repository.list_sessions().expect("list sessions");
    let keys: Vec<u64> = sessions
        .entries
        .iter()
        .map(|entry| entry.session_key)
        .collect();
    assert_eq!(keys, vec![first_key, second_key]);
    assert!(sessions.anomalies.is_empty());

    let listing = repository
        .list_snapshots(first_key)
        .expect("list first session snapshots");
    let sequence: Vec<u64> = listing
        .entries
        .iter()
        .map(|entry| entry.creation_sequence)
        .collect();
    assert_eq!(sequence, vec![1, 2, 3]);
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|entry| entry.snapshot_key)
            .collect::<Vec<_>>(),
        first_snapshots
            .iter()
            .map(|entry| entry.snapshot_key)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        repository.list_snapshots(first_key).expect("second list"),
        listing,
        "listing must be deterministic across calls"
    );
}

#[test]
fn invalid_entries_are_anomalies_not_silent_skips() {
    let root = temp_root("anomalies");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key) =
        create_session(&repository, &mut manager, None, DurableLineage::root());
    step(&mut manager, runtime, 1);
    let first = save_snapshot(&repository, &mut manager, session_key, runtime);
    step(&mut manager, runtime, 1);
    let second = save_snapshot(&repository, &mut manager, session_key, runtime);
    fs::create_dir(root.join("v1/sessions/session-x")).expect("create invalid session directory");
    let corrupt_path = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{}/snapshot.cfmeta",
        second.snapshot_key
    ));
    let mut corrupt = fs::read(&corrupt_path).expect("read metadata");
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    fs::write(&corrupt_path, &corrupt).expect("corrupt metadata");

    let sessions = repository.list_sessions().expect("list sessions");
    assert!(
        sessions
            .entries
            .iter()
            .any(|entry| entry.session_key == session_key)
    );
    assert!(sessions.anomalies.iter().any(|anomaly| {
        anomaly.path == root.join("v1/sessions/session-x")
            && matches!(anomaly.kind, StorageAnomalyKind::InvalidKey)
    }));

    let snapshots = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(snapshots.entries.len(), 1);
    assert_eq!(snapshots.entries[0].snapshot_key, first.snapshot_key);
    assert!(snapshots.anomalies.iter().any(|anomaly| {
        anomaly.path == corrupt_path.parent().expect("parent")
            && matches!(
                &anomaly.kind,
                StorageAnomalyKind::CorruptObject(SessionError::CorruptMetadata(_))
            )
    }));
    assert_eq!(
        fs::read(&corrupt_path).expect("re-read corrupt metadata"),
        corrupt,
        "listing must never repair an anomalous record"
    );
}

#[test]
fn open_capture_close_and_restart_delete_nothing() {
    let root = temp_root("no-gc");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key) =
        create_session(&repository, &mut manager, None, DurableLineage::root());
    step(&mut manager, runtime, 2);
    let entry = save_snapshot(&repository, &mut manager, session_key, runtime);
    let before = tree(&root);

    for _ in 0..2 {
        let reopened = SnapshotRepository::open(&root).expect("reopen root");
        reopened.list_sessions().expect("list sessions");
        reopened
            .list_snapshots(session_key)
            .expect("list snapshots");
        reopened
            .load_snapshot(session_key, entry.snapshot_key)
            .expect("load snapshot");
        drop(reopened);
    }
    drop(manager);
    assert_eq!(
        tree(&root),
        before,
        "open, capture, close and reopen must delete no committed data"
    );
}

#[test]
fn delete_is_explicit_target_only_and_non_cascading() {
    let root = temp_root("delete-target");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (first_runtime, first_key) = create_session(
        &repository,
        &mut manager,
        Some("first"),
        DurableLineage::root(),
    );
    step(&mut manager, first_runtime, 1);
    let first = save_snapshot(&repository, &mut manager, first_key, first_runtime);
    step(&mut manager, first_runtime, 1);
    let second = save_snapshot(&repository, &mut manager, first_key, first_runtime);
    let (second_runtime, second_key) = create_session(
        &repository,
        &mut manager,
        Some("second"),
        DurableLineage::root(),
    );
    step(&mut manager, second_runtime, 1);
    let other = save_snapshot(&repository, &mut manager, second_key, second_runtime);
    let child_lineage = DurableLineage {
        parent_session_key: Some(first_key),
        origin_snapshot_key: Some(first.snapshot_key),
    };
    let (_, child_key) = create_session(&repository, &mut manager, Some("child"), child_lineage);
    let child_before = repository.session(child_key).expect("child session record");
    let other_bytes = fs::read(root.join(format!(
        "v1/sessions/session-{second_key}/snapshots/snapshot-{}/checkpoint.cfsnap",
        other.snapshot_key
    )))
    .expect("read other checkpoint");

    repository
        .delete_snapshot(first_key, first.snapshot_key)
        .expect("delete one snapshot");
    assert!(
        repository
            .list_snapshots(first_key)
            .expect("list first")
            .entries
            .iter()
            .any(|entry| entry.snapshot_key == second.snapshot_key)
    );
    assert!(
        repository
            .list_snapshots(first_key)
            .expect("list first again")
            .entries
            .iter()
            .all(|entry| entry.snapshot_key != first.snapshot_key)
    );
    assert_eq!(
        repository
            .session(child_key)
            .expect("child record after delete"),
        child_before,
        "deleting an origin must not rewrite child lineage"
    );
    assert_eq!(
        fs::read(root.join(format!(
            "v1/sessions/session-{second_key}/snapshots/snapshot-{}/checkpoint.cfsnap",
            other.snapshot_key
        )))
        .expect("re-read other checkpoint"),
        other_bytes
    );
    assert_eq!(
        repository
            .list_sessions()
            .expect("list sessions")
            .entries
            .len(),
        3,
        "deleting a snapshot must not delete any session"
    );
}

#[test]
fn delete_session_with_snapshots_is_rejected() {
    let root = temp_root("delete-session");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key) =
        create_session(&repository, &mut manager, None, DurableLineage::root());
    step(&mut manager, runtime, 1);
    let entry = save_snapshot(&repository, &mut manager, session_key, runtime);
    assert_eq!(
        repository
            .delete_session(session_key)
            .expect_err("non-empty session"),
        SessionError::SessionNotEmpty(session_key)
    );
    assert!(
        root.join(format!("v1/sessions/session-{session_key}"))
            .exists()
    );
    repository
        .delete_snapshot(session_key, entry.snapshot_key)
        .expect("delete snapshot explicitly");
    repository
        .delete_session(session_key)
        .expect("delete empty session");
    assert!(
        !root
            .join(format!("v1/sessions/session-{session_key}"))
            .exists(),
        "the deleted session directory must be gone"
    );
    assert!(
        repository
            .list_sessions()
            .expect("list sessions")
            .entries
            .is_empty()
    );
}

#[test]
fn existing_key_is_a_conflict_not_an_overwrite() {
    let root = temp_root("conflicts");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (runtime, session_key) =
        create_session(&repository, &mut manager, None, DurableLineage::root());
    step(&mut manager, runtime, 1);
    save_snapshot(&repository, &mut manager, session_key, runtime);
    let occupied = root.join("v1/sessions/session-2");
    fs::create_dir(&occupied).expect("create occupied session directory");
    fs::write(occupied.join("sentinel"), b"occupied").expect("write sentinel");
    assert_eq!(
        repository
            .create_session(SessionId::from_raw(1), None, DurableLineage::root())
            .expect_err("occupied session key"),
        SessionError::SessionConflict(2)
    );
    assert_eq!(
        fs::read(occupied.join("sentinel")).expect("read sentinel"),
        b"occupied",
        "an existing session directory must never be overwritten"
    );

    let snapshot_occupied = root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-2"
    ));
    fs::create_dir(&snapshot_occupied).expect("create occupied snapshot directory");
    fs::write(snapshot_occupied.join("sentinel"), b"occupied").expect("write sentinel");
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    assert_eq!(
        repository
            .save_snapshot(session_key, snapshot, None)
            .expect_err("occupied snapshot key"),
        SessionError::SnapshotExists {
            session: session_key,
            snapshot: 2,
        }
    );
    assert_eq!(
        fs::read(snapshot_occupied.join("sentinel")).expect("read sentinel"),
        b"occupied",
        "an existing snapshot directory must never be overwritten"
    );
}
