#![allow(clippy::expect_used)]

//! Persistence isolation tests.
//!
//! Storage adds no hidden global mutable registry: each rehydrated session owns its
//! own backend, publishing or deleting one object never changes another object, and
//! independent readers are side-effect free.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId, SnapshotId};
use chronofork_sessions::{DurableLineage, SessionManager, SnapshotEntry, SnapshotRepository};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-isolation-{}-{name}",
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

fn publish_session(
    repository: &SnapshotRepository,
    manager: &mut SessionManager,
    name: Option<&str>,
    input: u32,
    steps: usize,
) -> (SessionId, u64, SnapshotEntry) {
    let runtime = manager.create_session().expect("runtime session");
    step(manager, runtime, input, steps);
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

fn checkpoint_path(root: &Path, session_key: u64, snapshot_key: u64) -> PathBuf {
    root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{snapshot_key}/checkpoint.cfsnap"
    ))
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
fn rehydrated_sessions_do_not_share_mutable_state() {
    let root = temp_root("mutable");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, first_key, first) = publish_session(&repository, &mut manager, Some("first"), 1, 3);
    let (_, second_key, second) = publish_session(&repository, &mut manager, Some("second"), 2, 7);
    drop(manager);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let first_loaded = repository
        .load_snapshot(first_key, first.snapshot_key)
        .expect("load first");
    let second_loaded = repository
        .load_snapshot(second_key, second.snapshot_key)
        .expect("load second");
    let mut manager = new_manager();
    let first_runtime = manager
        .create_session_from_stored_snapshot(first_loaded.snapshot, first_key)
        .expect("rehydrate first");
    let second_runtime = manager
        .create_session_from_stored_snapshot(second_loaded.snapshot, second_key)
        .expect("rehydrate second");
    let second_before = manager.inspect(second_runtime).expect("inspect second");
    let second_bytes = fs::read(checkpoint_path(&root, second_key, second.snapshot_key))
        .expect("read second checkpoint before stepping");
    step(&mut manager, first_runtime, 21, 4);
    let first_after = manager.inspect(first_runtime).expect("inspect first");
    let second_after = manager.inspect(second_runtime).expect("inspect second");
    assert_eq!(second_before.machine.tick, second_after.machine.tick);
    assert_eq!(
        second_before.machine.state_hash,
        second_after.machine.state_hash
    );
    assert_ne!(
        first_after.machine.state_hash,
        second_after.machine.state_hash
    );
    assert_eq!(
        fs::read(checkpoint_path(&root, second_key, second.snapshot_key))
            .expect("read second checkpoint after stepping"),
        second_bytes,
        "advancing a rehydrated session must not rewrite another checkpoint"
    );
    assert_eq!(
        repository
            .load_snapshot(second_key, second.snapshot_key)
            .expect("reload second")
            .snapshot
            .state_hash(),
        second.state_hash
    );
}

#[test]
fn publishing_a_child_does_not_mutate_the_parent_checkpoint() {
    let root = temp_root("child");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, parent_key, parent) = publish_session(&repository, &mut manager, Some("parent"), 3, 5);
    let parent_path = checkpoint_path(&root, parent_key, parent.snapshot_key);
    let parent_bytes = fs::read(&parent_path).expect("read parent checkpoint");
    drop(manager);

    let repository = SnapshotRepository::open(&root).expect("reopen root");
    let loaded = repository
        .load_snapshot(parent_key, parent.snapshot_key)
        .expect("load parent");
    let mut manager = new_manager();
    let parent_runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, parent_key)
        .expect("rehydrate parent");
    let child_runtime = manager
        .fork_from_snapshot(parent_runtime, SnapshotId::from_raw(parent.snapshot_key))
        .expect("fork child");
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
    let snapshot_id = manager.snapshot(child_runtime).expect("child checkpoint");
    let snapshot = manager
        .snapshot_record(child_runtime, snapshot_id)
        .expect("child checkpoint record");
    repository
        .save_snapshot(child_session.session_key, snapshot, Some("child"))
        .expect("publish child");
    assert_eq!(
        fs::read(&parent_path).expect("re-read parent checkpoint"),
        parent_bytes,
        "publishing a child must not mutate the parent checkpoint"
    );
    let parent_listing = repository
        .list_snapshots(parent_key)
        .expect("list parent snapshots");
    assert_eq!(parent_listing.entries.len(), 1);
    assert_eq!(parent_listing.entries[0].snapshot_key, parent.snapshot_key);
}

#[test]
fn deleting_one_snapshot_leaves_other_sessions_unchanged() {
    let root = temp_root("delete");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, first_key, first) = publish_session(&repository, &mut manager, Some("first"), 1, 2);
    let (_, second_key, second) = publish_session(&repository, &mut manager, Some("second"), 2, 6);
    let second_path = checkpoint_path(&root, second_key, second.snapshot_key);
    let second_bytes = fs::read(&second_path).expect("read second checkpoint");
    let second_entry = repository
        .load_snapshot(second_key, second.snapshot_key)
        .expect("load second")
        .entry;

    repository
        .delete_snapshot(first_key, first.snapshot_key)
        .expect("delete first snapshot");
    assert!(
        !checkpoint_path(&root, first_key, first.snapshot_key).exists(),
        "the deleted checkpoint must be gone"
    );
    assert!(
        repository
            .list_snapshots(first_key)
            .expect("list first")
            .entries
            .is_empty()
    );
    assert_eq!(
        fs::read(&second_path).expect("read second checkpoint after delete"),
        second_bytes
    );
    let second_after = repository
        .load_snapshot(second_key, second.snapshot_key)
        .expect("load second after delete")
        .entry;
    assert_eq!(second_after, second_entry);
    assert_eq!(
        repository
            .list_sessions()
            .expect("list sessions")
            .entries
            .len(),
        2,
        "deleting one snapshot must not delete either session"
    );
}

#[test]
fn independent_readers_are_side_effect_free() {
    let root = temp_root("readers");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, session_key, entry) = publish_session(&repository, &mut manager, Some("reader"), 4, 3);
    let before = tree(&root);

    let first = SnapshotRepository::open(&root).expect("open first reader");
    let second = SnapshotRepository::open(&root).expect("open second reader");
    let first_entry = first
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("first reader loads");
    let second_entry = second
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("second reader loads");
    assert_eq!(first_entry.entry, second_entry.entry);
    assert_eq!(
        first.list_sessions().expect("first list"),
        second.list_sessions().expect("second list")
    );
    assert_eq!(tree(&root), before, "readers must not write to the root");
}

#[test]
fn a_held_writer_lock_is_local_to_its_session_and_readers_stay_lock_free() {
    let root = temp_root("writer-lock-isolation");
    let repository = SnapshotRepository::initialize(&root).expect("initialize root");
    let mut manager = new_manager();
    let (_, first_key, first) = publish_session(&repository, &mut manager, Some("first"), 1, 3);
    let (_, second_key, second) = publish_session(&repository, &mut manager, Some("second"), 2, 5);
    let first_path = checkpoint_path(&root, first_key, first.snapshot_key);
    let first_bytes = fs::read(&first_path).expect("read first checkpoint");

    let reader = SnapshotRepository::open(&root).expect("open reader");
    let held = repository
        .acquire_writer_lock(first_key)
        .expect("hold the first session writer lock");
    let loaded_first = reader
        .load_snapshot(first_key, first.snapshot_key)
        .expect("a reader of the locked session takes no lock");
    assert_eq!(loaded_first.entry.state_hash, first.state_hash);
    let loaded_second = reader
        .load_snapshot(second_key, second.snapshot_key)
        .expect("a different session is not blocked");
    assert_eq!(loaded_second.entry.state_hash, second.state_hash);

    let mut rehydrated = new_manager();
    let runtime = rehydrated
        .create_session_from_stored_snapshot(loaded_second.snapshot, second_key)
        .expect("rehydrate the other session while the first writer lock is held");
    let child = rehydrated
        .fork_from_snapshot(runtime, SnapshotId::from_raw(second.snapshot_key))
        .expect("fork the other session");
    let child_session = repository
        .create_session(
            SessionId::from_raw(child.raw()),
            Some("child"),
            DurableLineage {
                parent_session_key: Some(second_key),
                origin_snapshot_key: Some(second.snapshot_key),
            },
        )
        .expect("session creation is root-lock scoped, not globally locked");
    rehydrated
        .attach_storage_session(child, child_session.session_key)
        .expect("attach child key");
    let snapshot_id = rehydrated.snapshot(child).expect("child checkpoint");
    let snapshot = rehydrated
        .snapshot_record(child, snapshot_id)
        .expect("child checkpoint record");
    repository
        .save_snapshot(child_session.session_key, snapshot, Some("child"))
        .expect("publish a child while the first session writer lock is held");
    assert!(
        !root.join("v1/state/allocation.lock").exists(),
        "the root allocation lock is short-lived and never left behind"
    );

    assert_eq!(
        fs::read(&first_path).expect("re-read first checkpoint"),
        first_bytes,
        "ownership metadata never changes canonical checkpoint bytes"
    );
    repository.release_writer_lock(&held).expect("release");
}
