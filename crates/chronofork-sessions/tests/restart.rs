#![allow(clippy::expect_used)]

//! Durable restart and reopen tests for the snapshot repository.
//!
//! The real cross-process boundary is exercised by the CLI acceptance tests. These
//! tests cover the reopen complement: every handle is dropped, a fresh repository
//! and manager open the same root, and the committed checkpoint must reproduce the
//! same tick and state hash without reusing any in-process handle.

use std::path::PathBuf;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{
    DurableLineage, SessionManager, SnapshotEntry, SnapshotRepository, StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-restart-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn step(manager: &mut SessionManager, session: SessionId, input: u32, count: usize) {
    for _ in 0..count {
        manager
            .step(session, InputFrame::new(input))
            .expect("session step succeeds");
    }
}

fn save_new_session(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    name: Option<&str>,
    steps: usize,
) -> (SessionId, u64, SnapshotEntry) {
    let runtime = manager.create_session().expect("runtime session");
    step(manager, runtime, 7, steps);
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
        .save_snapshot(session.session_key, &snapshot, name)
        .expect("publish snapshot");
    (runtime, session.session_key, entry)
}

#[test]
fn reopen_after_writer_close_round_trips() {
    let root = temp_root("round-trip");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = save_new_session(&repository, &mut manager, Some("baseline"), 7);
    let loaded_before = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load before close");
    let tick = loaded_before.snapshot.tick();
    let state_hash = loaded_before.snapshot.state_hash();
    let checkpoint_path = root.join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    let stored_bytes = std::fs::read(&checkpoint_path).expect("read checkpoint file");
    drop(manager);
    drop(repository);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let loaded = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load after reopen");
    assert_eq!(loaded.snapshot.tick(), tick);
    assert_eq!(loaded.snapshot.state_hash(), state_hash);
    assert_eq!(
        loaded.snapshot.to_bytes().expect("re-serialize checkpoint"),
        stored_bytes,
        "reopened checkpoint must re-serialize to the exact stored bytes"
    );
    assert_eq!(loaded.entry.name.as_deref(), Some("baseline"));

    let mut manager = new_manager();
    let runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, session_key)
        .expect("rehydrate session");
    let inspection = manager
        .inspect(runtime)
        .expect("inspect rehydrated session");
    assert_eq!(inspection.machine.tick, tick);
    assert_eq!(inspection.machine.state_hash, state_hash);
    assert_eq!(manager.durable_session_key(runtime), Some(session_key));
}

#[test]
fn continue_and_save_appends_snapshot_with_greater_sequence() {
    let root = temp_root("continue");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_runtime, session_key, first) =
        save_new_session(&repository, &mut manager, Some("first"), 4);
    let first_path = root.join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    let first_bytes = std::fs::read(&first_path).expect("read first checkpoint");
    drop(manager);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let latest = repository
        .latest_snapshot(session_key)
        .expect("list snapshots")
        .expect("a committed snapshot");
    assert_eq!(latest.snapshot_key, first.snapshot_key);
    let loaded = repository
        .load_snapshot(session_key, latest.snapshot_key)
        .expect("load latest");
    let mut manager = new_manager();
    let runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, session_key)
        .expect("rehydrate session");
    step(&mut manager, runtime, 11, 3);
    let snapshot_id = manager.snapshot(runtime).expect("second checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("second checkpoint record");
    let second = repository
        .save_snapshot(session_key, &snapshot, Some("second"))
        .expect("append snapshot");
    assert!(second.snapshot_key > first.snapshot_key);
    assert!(second.creation_sequence > first.creation_sequence);
    assert_eq!(
        std::fs::read(&first_path).expect("re-read first checkpoint"),
        first_bytes,
        "append must not rewrite an existing checkpoint"
    );

    drop(manager);
    drop(repository);
    let repository = SnapshotRepository::open(&root).expect("second reopen");
    let listing = repository
        .list_snapshots(session_key)
        .expect("list appended snapshots");
    assert!(listing.anomalies.is_empty());
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[0].snapshot_key, first.snapshot_key);
    assert_eq!(listing.entries[1].snapshot_key, second.snapshot_key);
    assert!(
        listing.entries[0].creation_sequence < listing.entries[1].creation_sequence,
        "listing must be ordered by logical creation sequence"
    );
}

#[test]
fn no_partial_object_is_visible_before_publish() {
    let root = temp_root("partial");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = save_new_session(&repository, &mut manager, None, 2);

    let staging = root.join("v1/sessions/.staging-session-99-0");
    std::fs::create_dir(&staging).expect("create staging directory");
    std::fs::write(staging.join("session.cfmeta"), b"partial").expect("write partial record");
    let trash = root.join("v1/sessions/session-1/snapshots/.trash-snapshot-7-0");
    std::fs::create_dir(&trash).expect("create trash directory");

    let sessions = repository.list_sessions().expect("list sessions");
    assert_eq!(sessions.entries.len(), 1);
    assert_eq!(sessions.entries[0].session_key, session_key);
    assert!(
        sessions
            .anomalies
            .iter()
            .any(|anomaly| matches!(anomaly.kind, StorageAnomalyKind::IncompletePublish))
    );

    let snapshots = repository
        .list_snapshots(session_key)
        .expect("list snapshots");
    assert_eq!(snapshots.entries.len(), 1);
    assert_eq!(snapshots.entries[0].snapshot_key, entry.snapshot_key);
    assert!(
        snapshots
            .anomalies
            .iter()
            .any(|anomaly| matches!(anomaly.kind, StorageAnomalyKind::PendingDeletion))
    );
    assert!(
        repository.load_snapshot(99, 1).is_err(),
        "a staging directory must never be loaded as a session"
    );
}
