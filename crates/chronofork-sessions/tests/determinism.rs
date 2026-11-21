#![allow(clippy::expect_used)]

//! Determinism tests for durable checkpoints.
//!
//! A stored checkpoint is exactly the canonical envelope. Reopening it, replaying
//! from it and changing non-canonical metadata must reproduce the same tick, state
//! hash and checkpoint bytes. Host timestamps never enter canonical data.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId, StateHash, Tick};
use chronofork_sessions::{
    DurableLineage, SessionManager, Snapshot, SnapshotEntry, SnapshotRepository,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-determinism-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temporary root");
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

fn setup(name: &str) -> (PathBuf, u64, SnapshotEntry) {
    let root = temp_root(name);
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 5, 6);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("deterministic"),
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
        .save_snapshot(session.session_key, &snapshot, Some("checkpoint"))
        .expect("publish snapshot");
    (root, session.session_key, entry)
}

fn checkpoint_path(root: &Path, session_key: u64, snapshot_key: u64) -> PathBuf {
    root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{snapshot_key}/checkpoint.cfsnap"
    ))
}

struct Report {
    tick: Tick,
    state_hash: StateHash,
    checkpoint_sha256: StateHash,
}

fn reopen_report(root: &Path, session_key: u64, snapshot_key: u64) -> Report {
    let repository = SnapshotRepository::open(root).expect("reopen root");
    let loaded = repository
        .load_snapshot(session_key, snapshot_key)
        .expect("load snapshot");
    let mut manager = new_manager();
    let runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, session_key)
        .expect("rehydrate session");
    let inspection = manager
        .inspect(runtime)
        .expect("inspect rehydrated session");
    Report {
        tick: inspection.machine.tick,
        state_hash: inspection.machine.state_hash,
        checkpoint_sha256: loaded.entry.checkpoint_sha256,
    }
}

#[test]
fn stored_checkpoint_bytes_equal_canonical_envelope() {
    let (root, session_key, entry) = setup("canonical");
    let path = checkpoint_path(&root, session_key, entry.snapshot_key);
    let bytes = fs::read(&path).expect("read checkpoint");
    let decoded = Snapshot::from_bytes(&bytes).expect("decode stored envelope");
    assert_eq!(
        decoded.to_bytes().expect("re-serialize envelope"),
        bytes,
        "the file must be exactly Snapshot::to_bytes output"
    );
    assert_eq!(decoded.id().raw(), entry.snapshot_key);
    assert_eq!(decoded.tick(), entry.tick);
    assert_eq!(decoded.state_hash(), entry.state_hash);
    assert_eq!(chronofork_core::hash_bytes(&bytes), entry.checkpoint_sha256);
}

#[test]
fn restart_reproduces_tick_and_state_hash() {
    let (root, session_key, entry) = setup("restart");
    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let loaded = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load snapshot");
    let tick = loaded.snapshot.tick();
    let state_hash = loaded.snapshot.state_hash();

    let mut first = new_manager();
    let first_runtime = first
        .create_session_from_stored_snapshot(loaded.snapshot.clone(), session_key)
        .expect("rehydrate first");
    let first_inspection = first.inspect(first_runtime).expect("inspect first");
    assert_eq!(first_inspection.machine.tick, tick);
    assert_eq!(first_inspection.machine.state_hash, state_hash);

    let mut second = new_manager();
    let second_runtime = second
        .create_session_from_stored_snapshot(loaded.snapshot, session_key)
        .expect("rehydrate second");
    step(&mut first, first_runtime, 13, 5);
    step(&mut second, second_runtime, 13, 5);
    let first_after = first
        .inspect(first_runtime)
        .expect("inspect first after replay");
    let second_after = second
        .inspect(second_runtime)
        .expect("inspect second after replay");
    assert_eq!(first_after.machine.tick, second_after.machine.tick);
    assert_eq!(
        first_after.machine.state_hash,
        second_after.machine.state_hash
    );
    assert_eq!(
        first_after.machine.tick,
        Tick::from_raw(tick.raw() + 5),
        "five deterministic steps advance the tick exactly"
    );
}

#[test]
fn metadata_changes_do_not_change_checkpoint_bytes_or_state_hash() {
    let (root, session_key, entry) = setup("metadata");
    let path = checkpoint_path(&root, session_key, entry.snapshot_key);
    let before = fs::read(&path).expect("read checkpoint");
    let before_hash = chronofork_core::hash_bytes(&before);
    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let loaded_before = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load before rename");
    let tick = loaded_before.snapshot.tick();
    let state_hash = loaded_before.snapshot.state_hash();

    let renamed = repository
        .rename_snapshot(session_key, entry.snapshot_key, Some("renamed"))
        .expect("rename snapshot");
    assert_eq!(renamed.name.as_deref(), Some("renamed"));
    assert_eq!(
        fs::read(&path).expect("re-read checkpoint"),
        before,
        "renaming must not change checkpoint bytes"
    );
    let loaded_after = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load after rename");
    assert_eq!(loaded_after.snapshot.tick(), tick);
    assert_eq!(loaded_after.snapshot.state_hash(), state_hash);
    assert_eq!(
        chronofork_core::hash_bytes(&fs::read(&path).expect("read checkpoint")),
        before_hash
    );

    repository
        .rename_snapshot(session_key, entry.snapshot_key, None)
        .expect("clear snapshot name");
    assert_eq!(
        fs::read(&path).expect("read checkpoint after clearing name"),
        before
    );
}

#[test]
fn two_restarts_produce_identical_reports() {
    let (root, session_key, entry) = setup("two-restarts");
    let first = reopen_report(&root, session_key, entry.snapshot_key);
    let second = reopen_report(&root, session_key, entry.snapshot_key);
    assert_eq!(first.tick, second.tick);
    assert_eq!(first.state_hash, second.state_hash);
    assert_eq!(first.checkpoint_sha256, second.checkpoint_sha256);
    assert_eq!(first.tick, entry.tick);
    assert_eq!(first.state_hash, entry.state_hash);
}
