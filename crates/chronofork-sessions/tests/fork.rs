#![allow(clippy::expect_used)]

//! Durable fork-after-reopen tests.
//!
//! Rehydration uses the same validate-then-restore path as any other checkpoint.
//! A fork publishes under a new durable session key, records the durable parent
//! session and origin snapshot, and never copies or mutates the parent checkpoint.

use std::path::PathBuf;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId, SnapshotId};
use chronofork_sessions::{DurableLineage, SessionManager, SnapshotEntry, SnapshotRepository};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-fork-{}-{name}",
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

fn save_snapshot(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    session_key: u64,
    runtime: SessionId,
    name: Option<&str>,
) -> SnapshotEntry {
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    repository
        .save_snapshot(session_key, &snapshot, name)
        .expect("publish snapshot")
}

fn publish_parent(root: &PathBuf) -> (SnapshotRepository, SnapshotEntry) {
    let repository = SnapshotRepository::initialize(root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("parent runtime session");
    step(&mut manager, runtime, 9, 5);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("parent"),
            DurableLineage::root(),
        )
        .expect("durable parent session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach parent key");
    let entry = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("origin"),
    );
    (repository, entry)
}

fn rehydrate_and_fork(
    repository: &SnapshotRepository,
    parent: &SnapshotEntry,
) -> (SessionManager, SessionId, SessionId) {
    let loaded = repository
        .load_snapshot(parent.session_key, parent.snapshot_key)
        .expect("load parent snapshot");
    let mut manager = new_manager();
    let parent_runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, parent.session_key)
        .expect("rehydrate parent");
    let child_runtime = manager
        .fork_from_snapshot(parent_runtime, SnapshotId::from_raw(parent.snapshot_key))
        .expect("fork child");
    (manager, parent_runtime, child_runtime)
}

#[test]
fn fork_after_reopen_preserves_lineage_tick_and_hash() {
    let root = temp_root("lineage");
    let (repository, parent) = publish_parent(&root);
    let parent_checkpoint =
        root.join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    let parent_bytes = std::fs::read(&parent_checkpoint).expect("read parent checkpoint");
    drop(repository);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let (mut manager, parent_runtime, child_runtime) = rehydrate_and_fork(&repository, &parent);
    let parent_inspection = manager.inspect(parent_runtime).expect("inspect parent");
    let child_inspection = manager.inspect(child_runtime).expect("inspect child");
    assert_eq!(
        child_inspection.machine.tick,
        parent_inspection.machine.tick
    );
    assert_eq!(
        child_inspection.machine.state_hash,
        parent_inspection.machine.state_hash
    );
    assert_eq!(parent_inspection.machine.tick, parent.tick);
    assert_eq!(parent_inspection.machine.state_hash, parent.state_hash);

    let parent_key = manager
        .durable_session_key(parent_runtime)
        .expect("parent durable key");
    assert_eq!(parent_key, parent.session_key);
    let lineage = DurableLineage {
        parent_session_key: Some(parent_key),
        origin_snapshot_key: Some(parent.snapshot_key),
    };
    let child_session = repository
        .create_session(
            SessionId::from_raw(child_runtime.raw()),
            Some("child"),
            lineage,
        )
        .expect("durable child session");
    manager
        .attach_storage_session(child_runtime, child_session.session_key)
        .expect("attach child key");
    let child = save_snapshot(
        &repository,
        &mut manager,
        child_session.session_key,
        child_runtime,
        Some("child-checkpoint"),
    );
    assert_eq!(child.parent_session_key, Some(parent_key));
    assert_eq!(child.origin_snapshot_key, Some(parent.snapshot_key));
    assert_eq!(
        std::fs::read(&parent_checkpoint).expect("re-read parent checkpoint"),
        parent_bytes,
        "publishing a child must not mutate the parent checkpoint"
    );
}

#[test]
fn fork_publishes_under_child_session_key_without_copying_parent_checkpoint() {
    let root = temp_root("child-key");
    let (repository, parent) = publish_parent(&root);
    let (mut manager, parent_runtime, child_runtime) = rehydrate_and_fork(&repository, &parent);
    let parent_key = manager
        .durable_session_key(parent_runtime)
        .expect("parent durable key");
    let child_session = repository
        .create_session(
            SessionId::from_raw(child_runtime.raw()),
            Some("child"),
            DurableLineage {
                parent_session_key: Some(parent_key),
                origin_snapshot_key: Some(parent.snapshot_key),
            },
        )
        .expect("durable child session");
    assert_ne!(child_session.session_key, parent_key);
    manager
        .attach_storage_session(child_runtime, child_session.session_key)
        .expect("attach child key");
    let child = save_snapshot(
        &repository,
        &mut manager,
        child_session.session_key,
        child_runtime,
        None,
    );
    assert_eq!(child.snapshot_key, 1);
    let parent_path = root.join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    let child_path = root.join(format!(
        "v1/sessions/session-{}/snapshots/snapshot-{}/checkpoint.cfsnap",
        child.session_key, child.snapshot_key
    ));
    let parent_bytes = std::fs::read(&parent_path).expect("read parent checkpoint");
    let child_bytes = std::fs::read(&child_path).expect("read child checkpoint");
    assert_ne!(
        parent_bytes, child_bytes,
        "a child checkpoint is its own canonical envelope, not a copy"
    );
    assert_eq!(
        repository
            .load_snapshot(parent.session_key, parent.snapshot_key)
            .expect("reload parent")
            .snapshot
            .tick(),
        parent.tick
    );
}

#[test]
fn lineage_survives_a_second_reopen() {
    let root = temp_root("second-reopen");
    let (repository, parent) = publish_parent(&root);
    let (mut manager, parent_runtime, child_runtime) = rehydrate_and_fork(&repository, &parent);
    let parent_key = manager
        .durable_session_key(parent_runtime)
        .expect("parent durable key");
    let child_session = repository
        .create_session(
            SessionId::from_raw(child_runtime.raw()),
            Some("child"),
            DurableLineage {
                parent_session_key: Some(parent_key),
                origin_snapshot_key: Some(parent.snapshot_key),
            },
        )
        .expect("durable child session");
    manager
        .attach_storage_session(child_runtime, child_session.session_key)
        .expect("attach child key");
    let child = save_snapshot(
        &repository,
        &mut manager,
        child_session.session_key,
        child_runtime,
        Some("child-checkpoint"),
    );
    drop(manager);
    drop(repository);

    for _ in 0..2 {
        let repository = SnapshotRepository::open(&root).expect("reopen root");
        let session = repository
            .session(child_session.session_key)
            .expect("child session record");
        assert_eq!(session.parent_session_key, Some(parent_key));
        assert_eq!(session.origin_snapshot_key, Some(parent.snapshot_key));
        let loaded = repository
            .load_snapshot(child_session.session_key, child.snapshot_key)
            .expect("load child snapshot");
        assert_eq!(loaded.entry.parent_session_key, Some(parent_key));
        assert_eq!(loaded.entry.origin_snapshot_key, Some(parent.snapshot_key));
    }
}
