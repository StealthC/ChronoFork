#![allow(clippy::expect_used)]

//! Durable metadata round-trip tests.
//!
//! Metadata is non-canonical: it carries names, durable keys, mirrored envelope
//! fields, lineage and advisory host timestamps, but changing it must never change
//! `checkpoint.cfsnap`, the state hash or a replay result.

use std::path::PathBuf;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId};
use chronofork_sessions::{DurableLineage, SessionManager, SnapshotRepository, StorageLimits};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-metadata-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn step(manager: &mut SessionManager, session: SessionId, count: usize) {
    for _ in 0..count {
        manager
            .step(session, InputFrame::new(5))
            .expect("session step succeeds");
    }
}

fn save_snapshot(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    session_key: u64,
    runtime: SessionId,
    name: Option<&str>,
) -> chronofork_sessions::SnapshotEntry {
    let snapshot_id = manager.snapshot(runtime).expect("checkpoint");
    let snapshot = manager
        .snapshot_record(runtime, snapshot_id)
        .expect("checkpoint record");
    repository
        .save_snapshot(session_key, snapshot, name)
        .expect("publish snapshot")
}

#[test]
fn named_snapshot_round_trips_metadata() {
    let root = temp_root("named");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 3);
    let lineage = DurableLineage {
        parent_session_key: Some(7),
        origin_snapshot_key: Some(3),
    };
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("named-session"),
            lineage,
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let entry = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("checkpoint-one"),
    );
    assert_eq!(entry.session_key, session.session_key);
    assert_eq!(entry.name.as_deref(), Some("checkpoint-one"));
    assert_eq!(entry.creation_sequence, 1);
    assert_eq!(entry.parent_session_key, Some(7));
    assert_eq!(entry.origin_snapshot_key, Some(3));
    drop(manager);
    drop(repository);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let session = repository
        .session(session.session_key)
        .expect("session record");
    assert_eq!(session.name.as_deref(), Some("named-session"));
    assert_eq!(session.parent_session_key, Some(7));
    assert_eq!(session.origin_snapshot_key, Some(3));
    let loaded = repository
        .load_snapshot(session.session_key, entry.snapshot_key)
        .expect("load named snapshot");
    assert_eq!(loaded.entry.name.as_deref(), Some("checkpoint-one"));
    assert_eq!(loaded.entry.creation_sequence, entry.creation_sequence);
    assert_eq!(loaded.entry.parent_session_key, Some(7));
    assert_eq!(loaded.entry.origin_snapshot_key, Some(3));
    let backend = loaded.snapshot.backend();
    assert_eq!(loaded.entry.tick, loaded.snapshot.tick());
    assert_eq!(loaded.entry.state_hash, loaded.snapshot.state_hash());
    assert_eq!(
        loaded.entry.checkpoint_length,
        u64::try_from(
            loaded
                .snapshot
                .to_bytes()
                .expect("re-serialize checkpoint")
                .len()
        )
        .expect("length fits u64")
    );
    assert_eq!(backend.backend_id().as_str(), "chronofork.test.synthetic");
    assert_eq!(backend.backend_version(), env!("CARGO_PKG_VERSION"));
    assert_eq!(
        loaded.entry.checkpoint_sha256,
        chronofork_core::hash_bytes(&loaded.snapshot.to_bytes().expect("re-serialize checkpoint"))
    );
}

#[test]
fn rename_preserves_checkpoint_bytes_and_state_hash() {
    let root = temp_root("rename");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 4);
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
    let entry = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("before"),
    );
    let checkpoint_path = root.join("v1/sessions/session-1/snapshots/snapshot-1/checkpoint.cfsnap");
    let metadata_path = root.join("v1/sessions/session-1/snapshots/snapshot-1/snapshot.cfmeta");
    let checkpoint_before = std::fs::read(&checkpoint_path).expect("read checkpoint");
    let metadata_before = std::fs::read(&metadata_path).expect("read metadata");
    let loaded_before = repository
        .load_snapshot(session.session_key, entry.snapshot_key)
        .expect("load before rename");
    let tick = loaded_before.snapshot.tick();
    let state_hash = loaded_before.snapshot.state_hash();

    let renamed = repository
        .rename_snapshot(session.session_key, entry.snapshot_key, Some("after"))
        .expect("rename snapshot");
    assert_eq!(renamed.name.as_deref(), Some("after"));
    assert_eq!(
        std::fs::read(&checkpoint_path).expect("re-read checkpoint"),
        checkpoint_before,
        "rename must not rewrite the checkpoint"
    );
    assert_ne!(
        std::fs::read(&metadata_path).expect("re-read metadata"),
        metadata_before,
        "rename must rewrite only the metadata record"
    );
    let loaded_after = repository
        .load_snapshot(session.session_key, entry.snapshot_key)
        .expect("load after rename");
    assert_eq!(loaded_after.snapshot.tick(), tick);
    assert_eq!(loaded_after.snapshot.state_hash(), state_hash);
    assert_eq!(loaded_after.entry.name.as_deref(), Some("after"));
}

#[test]
fn duplicate_names_are_labels_not_identities() {
    let root = temp_root("duplicates");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 2);
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
    let first = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("same-label"),
    );
    step(&mut manager, runtime, 2);
    let second = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("same-label"),
    );
    assert_ne!(first.snapshot_key, second.snapshot_key);
    assert_ne!(first.creation_sequence, second.creation_sequence);
    let listing = repository
        .list_snapshots(session.session_key)
        .expect("list snapshots");
    assert_eq!(listing.entries.len(), 2);
    assert!(
        listing
            .entries
            .iter()
            .all(|entry| entry.name.as_deref() == Some("same-label"))
    );
}

#[test]
fn continue_and_save_appended_snapshot_metadata_round_trips() {
    let root = temp_root("continue-metadata");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    step(&mut manager, runtime, 2);
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("continued"),
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let first = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("first"),
    );
    drop(manager);
    drop(repository);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let latest = repository
        .latest_snapshot(session.session_key)
        .expect("list latest")
        .expect("a committed snapshot");
    let loaded = repository
        .load_snapshot(session.session_key, latest.snapshot_key)
        .expect("load latest");
    let mut manager = new_manager();
    let runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, session.session_key)
        .expect("rehydrate");
    step(&mut manager, runtime, 3);
    let second = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("second"),
    );
    assert!(second.creation_sequence > first.creation_sequence);
    drop(manager);
    drop(repository);

    let repository = SnapshotRepository::open(&root).expect("second reopen");
    let loaded = repository
        .load_snapshot(session.session_key, second.snapshot_key)
        .expect("load appended snapshot");
    assert_eq!(loaded.entry.name.as_deref(), Some("second"));
    assert_eq!(loaded.entry.creation_sequence, second.creation_sequence);
    let first_again = repository
        .load_snapshot(session.session_key, first.snapshot_key)
        .expect("load first snapshot");
    assert_eq!(first_again.entry.name.as_deref(), Some("first"));
    assert_eq!(first_again.entry.creation_sequence, first.creation_sequence);
}

#[test]
fn lowered_limits_still_round_trip_metadata() {
    let root = temp_root("limits");
    let limits = StorageLimits {
        max_name_bytes: 16,
        ..StorageLimits::default()
    };
    let repository =
        SnapshotRepository::initialize_with_limits(&root, limits).expect("initialize root");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("runtime session");
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            Some("short-name"),
            DurableLineage::root(),
        )
        .expect("durable session");
    manager
        .attach_storage_session(runtime, session.session_key)
        .expect("attach durable key");
    let entry = save_snapshot(
        &repository,
        &mut manager,
        session.session_key,
        runtime,
        Some("tiny"),
    );
    let loaded = repository
        .load_snapshot(session.session_key, entry.snapshot_key)
        .expect("load with lowered limits");
    assert_eq!(loaded.entry.name.as_deref(), Some("tiny"));
}
