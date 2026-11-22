#![allow(clippy::expect_used)]

//! Typed rejection tests for the durable storage layer.
//!
//! Every corrupted, incompatible or identity-mismatched object must fail with a
//! specific error category, never panic and never repair or migrate the tree.
//! Backend compatibility stays in `validate_envelope`; storage only mirrors and
//! binds envelope fields.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_backend_api::{BackendId, BackendSnapshot};
use chronofork_core::{InputFrame, SessionId, SnapshotId, hash_bytes};
use chronofork_sessions::{
    DurableLineage, SNAPSHOT_FORMAT_VERSION, STORAGE_LAYOUT_VERSION,
    STORAGE_METADATA_FORMAT_VERSION, SessionError, SessionManager, Snapshot, SnapshotEntry,
    SnapshotRepository, StorageAnomalyKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chronofork-storage-rejection-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temporary root");
    root
}

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn step(manager: &mut SessionManager, session: SessionId, count: usize) {
    for _ in 0..count {
        manager
            .step(session, InputFrame::new(1))
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
        .save_snapshot(session_key, &snapshot, None)
        .expect("publish snapshot")
}

fn setup(name: &str) -> (PathBuf, SnapshotRepository, u64, SnapshotEntry) {
    let root = temp_root(name);
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
    let entry = save_snapshot(&repository, &mut manager, session.session_key, runtime);
    (root, repository, session.session_key, entry)
}

fn session_metadata_path(root: &Path, session_key: u64) -> PathBuf {
    root.join(format!("v1/sessions/session-{session_key}/session.cfmeta"))
}

fn snapshot_directory(root: &Path, session_key: u64, snapshot_key: u64) -> PathBuf {
    root.join(format!(
        "v1/sessions/session-{session_key}/snapshots/snapshot-{snapshot_key}"
    ))
}

fn rewrite_metadata_checksum(path: &Path) {
    let mut bytes = fs::read(path).expect("read metadata");
    let body_len = bytes.len() - 32;
    let checksum = hash_bytes(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(checksum.as_bytes());
    fs::write(path, bytes).expect("write metadata");
}

fn find_last(bytes: &[u8], needle: &[u8; 32]) -> usize {
    bytes
        .windows(32)
        .rposition(|window| window == needle)
        .expect("checkpoint SHA-256 is present in metadata")
}

fn rebind_metadata(path: &Path, old_sha256: &[u8; 32], new_length: u64, new_sha256: &[u8; 32]) {
    let mut bytes = fs::read(path).expect("read metadata");
    let position = find_last(&bytes, old_sha256);
    assert!(position >= 8, "checkpoint length precedes the checksum");
    bytes[position - 8..position].copy_from_slice(&new_length.to_le_bytes());
    bytes[position..position + 32].copy_from_slice(new_sha256);
    let body_len = bytes.len() - 32;
    let checksum = hash_bytes(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(checksum.as_bytes());
    fs::write(path, bytes).expect("write metadata");
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
fn root_and_layout_rejections_are_typed() {
    let root = temp_root("root");
    assert_eq!(
        SnapshotRepository::open(root.join("missing")).expect_err("missing root"),
        SessionError::NotInitialized
    );

    let empty = root.join("empty");
    fs::create_dir(&empty).expect("create empty directory");
    assert_eq!(
        SnapshotRepository::open(&empty).expect_err("uninitialized root"),
        SessionError::NotInitialized
    );

    let foreign = root.join("foreign");
    fs::create_dir(&foreign).expect("create foreign directory");
    fs::write(foreign.join("payload.bin"), b"data").expect("write foreign file");
    assert!(matches!(
        SnapshotRepository::initialize(&foreign).expect_err("non-empty root"),
        SessionError::RootNotEmpty(_)
    ));

    let wrong_magic = root.join("wrong-magic");
    fs::create_dir(&wrong_magic).expect("create wrong-magic directory");
    let mut marker = *b"CFSTORE1";
    marker[0] = b'X';
    let mut bytes = Vec::from(marker);
    bytes.extend_from_slice(&STORAGE_LAYOUT_VERSION.to_le_bytes());
    fs::write(wrong_magic.join("chronofork.store"), bytes).expect("write marker");
    assert!(matches!(
        SnapshotRepository::open(&wrong_magic).expect_err("wrong magic"),
        SessionError::NotAStorageRoot(_)
    ));

    let initialized = root.join("initialized");
    SnapshotRepository::initialize(&initialized).expect("initialize root");
    let marker_path = initialized.join("chronofork.store");
    let mut marker = fs::read(&marker_path).expect("read marker");
    marker[8..10].copy_from_slice(&(STORAGE_LAYOUT_VERSION + 1).to_le_bytes());
    fs::write(&marker_path, marker).expect("write marker");
    assert_eq!(
        SnapshotRepository::open(&initialized).expect_err("wrong layout version"),
        SessionError::UnsupportedLayoutVersion {
            found: STORAGE_LAYOUT_VERSION + 1,
            supported: STORAGE_LAYOUT_VERSION,
        }
    );

    let future = root.join("future");
    let repository = SnapshotRepository::initialize(&future).expect("initialize future root");
    fs::create_dir(future.join("v2")).expect("create future layout directory");
    fs::write(future.join("v2/session-1"), b"not v1 data").expect("write future data");
    let listing = repository.list_sessions().expect("list sessions");
    assert!(listing.entries.is_empty());
    assert!(
        listing
            .anomalies
            .iter()
            .any(|anomaly| matches!(anomaly.kind, StorageAnomalyKind::FutureLayoutVersion(2)))
    );
    assert!(
        !future.join("v1/sessions/session-1").exists(),
        "future layout data must never be adopted as v1"
    );
}

#[test]
fn metadata_corruption_and_version_rejections_are_typed() {
    let (root, repository, session_key, entry) = setup("metadata");
    let metadata_path =
        snapshot_directory(&root, session_key, entry.snapshot_key).join("snapshot.cfmeta");
    let original = fs::read(&metadata_path).expect("read metadata");

    let mut flipped = original.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0xff;
    fs::write(&metadata_path, &flipped).expect("write corrupt metadata");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("checksum corruption"),
        SessionError::CorruptMetadata(_)
    ));

    let mut trailing = original.clone();
    trailing.push(0);
    fs::write(&metadata_path, &trailing).expect("write trailing metadata");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("trailing bytes"),
        SessionError::CorruptMetadata(_)
    ));

    let mut version = original.clone();
    version[8..10].copy_from_slice(&(STORAGE_METADATA_FORMAT_VERSION + 1).to_le_bytes());
    fs::write(&metadata_path, &version).expect("write future metadata version");
    rewrite_metadata_checksum(&metadata_path);
    assert_eq!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("unsupported metadata version"),
        SessionError::UnsupportedMetadataVersion {
            found: STORAGE_METADATA_FORMAT_VERSION + 1,
            supported: STORAGE_METADATA_FORMAT_VERSION,
        }
    );

    let session_record =
        fs::read(session_metadata_path(&root, session_key)).expect("read session metadata");
    fs::write(&metadata_path, session_record).expect("write wrong record kind");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("record kind mismatch"),
        SessionError::MetadataMismatch(_)
    ));
}

#[test]
fn checkpoint_corruption_truncation_and_trailing_bytes_are_typed() {
    let (root, repository, session_key, entry) = setup("checkpoint");
    let directory = snapshot_directory(&root, session_key, entry.snapshot_key);
    let checkpoint_path = directory.join("checkpoint.cfsnap");
    let metadata_path = directory.join("snapshot.cfmeta");
    let checkpoint = fs::read(&checkpoint_path).expect("read checkpoint");
    let original_sha256 = *hash_bytes(&checkpoint).as_bytes();

    let mut corrupt = checkpoint.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    fs::write(&checkpoint_path, &corrupt).expect("write corrupt checkpoint");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("checkpoint checksum corruption"),
        SessionError::MetadataMismatch(_)
    ));
    assert!(matches!(
        Snapshot::from_bytes(&corrupt),
        Err(SessionError::InvalidSnapshot(_))
    ));

    let truncated = &checkpoint[..checkpoint.len() - 1];
    fs::write(&checkpoint_path, truncated).expect("write truncated checkpoint");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("checkpoint truncation"),
        SessionError::MetadataMismatch(_)
    ));
    assert!(matches!(
        Snapshot::from_bytes(truncated),
        Err(SessionError::InvalidSnapshot(_))
    ));

    let mut trailing = checkpoint.clone();
    trailing.push(0);
    fs::write(&checkpoint_path, &trailing).expect("write checkpoint trailing byte");
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("checkpoint trailing byte"),
        SessionError::MetadataMismatch(_)
    ));

    // Rebind the metadata so the corrupted checkpoint reaches Snapshot::from_bytes.
    let mut incompatible = checkpoint.clone();
    incompatible[8..10].copy_from_slice(&(SNAPSHOT_FORMAT_VERSION + 1).to_le_bytes());
    let body_len = incompatible.len() - 32;
    let checksum = hash_bytes(&incompatible[..body_len]);
    incompatible[body_len..].copy_from_slice(checksum.as_bytes());
    let incompatible_sha256 = *hash_bytes(&incompatible).as_bytes();
    fs::write(&checkpoint_path, &incompatible).expect("write incompatible checkpoint");
    rebind_metadata(
        &metadata_path,
        &original_sha256,
        u64::try_from(incompatible.len()).expect("length fits"),
        &incompatible_sha256,
    );
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("unsupported outer version"),
        SessionError::IncompatibleSnapshot(_)
    ));

    fs::write(&checkpoint_path, &trailing).expect("write trailing checkpoint");
    let trailing_sha256 = *hash_bytes(&trailing).as_bytes();
    rebind_metadata(
        &metadata_path,
        &incompatible_sha256,
        u64::try_from(trailing.len()).expect("length fits"),
        &trailing_sha256,
    );
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("trailing bytes reach the envelope reader"),
        SessionError::InvalidSnapshot(_)
    ));

    fs::write(&checkpoint_path, truncated).expect("write truncated checkpoint");
    let truncated_sha256 = *hash_bytes(truncated).as_bytes();
    rebind_metadata(
        &metadata_path,
        &trailing_sha256,
        u64::try_from(truncated.len()).expect("length fits"),
        &truncated_sha256,
    );
    assert!(matches!(
        repository
            .load_snapshot(session_key, entry.snapshot_key)
            .expect_err("truncation reaches the envelope reader"),
        SessionError::InvalidSnapshot(_)
    ));
}

#[test]
fn backend_compatibility_fields_are_rejected_once() {
    let (_root, repository, session_key, entry) = setup("compatibility");
    let loaded = repository
        .load_snapshot(session_key, entry.snapshot_key)
        .expect("load source snapshot");
    let backend = loaded.snapshot.backend().clone();
    let variants = [
        (
            "backend_id",
            BackendSnapshot::new(
                BackendId::new("chronofork.foreign"),
                backend.backend_version(),
                backend.format_version(),
                backend.tick(),
                backend.config_hash(),
                backend.state_hash(),
                backend.payload().to_vec(),
            ),
        ),
        (
            "backend_version",
            BackendSnapshot::new(
                backend.backend_id().clone(),
                "9.9.9",
                backend.format_version(),
                backend.tick(),
                backend.config_hash(),
                backend.state_hash(),
                backend.payload().to_vec(),
            ),
        ),
        (
            "backend_format_version",
            BackendSnapshot::new(
                backend.backend_id().clone(),
                backend.backend_version(),
                2,
                backend.tick(),
                backend.config_hash(),
                backend.state_hash(),
                backend.payload().to_vec(),
            ),
        ),
        (
            "config_hash",
            BackendSnapshot::new(
                backend.backend_id().clone(),
                backend.backend_version(),
                backend.format_version(),
                backend.tick(),
                chronofork_core::StateHash::from_bytes([0xAB; 32]),
                backend.state_hash(),
                backend.payload().to_vec(),
            ),
        ),
    ];

    for (field, variant) in variants {
        let snapshot = Snapshot::new(SnapshotId::from_raw(99), SessionId::from_raw(1), variant);
        let stored = repository
            .save_snapshot(session_key, &snapshot, None)
            .expect("storage stores an opaque backend envelope");
        let loaded = repository
            .load_snapshot(session_key, stored.snapshot_key)
            .expect("storage does not decide backend compatibility");
        let mut manager = new_manager();
        let error = manager
            .create_session_from_stored_snapshot(loaded.snapshot, session_key)
            .expect_err("the session restore path rejects incompatibility");
        assert!(
            matches!(&error, SessionError::IncompatibleSnapshot(message) if message.contains(field)),
            "expected an incompatibility naming {field}, got {error}"
        );
    }
}

#[test]
fn identity_and_metadata_binding_rejections_are_typed() {
    let (root, repository, session_key, first) = setup("identity");
    let mut manager = new_manager();
    let runtime = manager.create_session().expect("second runtime session");
    step(&mut manager, runtime, 3);
    manager
        .attach_storage_session(runtime, session_key)
        .expect("attach durable key");
    let second = save_snapshot(&repository, &mut manager, session_key, runtime);
    assert_ne!(first.snapshot_key, second.snapshot_key);

    let first_checkpoint =
        snapshot_directory(&root, session_key, first.snapshot_key).join("checkpoint.cfsnap");
    let second_checkpoint =
        snapshot_directory(&root, session_key, second.snapshot_key).join("checkpoint.cfsnap");
    let original = fs::read(&first_checkpoint).expect("read first checkpoint");
    fs::copy(&second_checkpoint, &first_checkpoint).expect("swap checkpoint");
    assert!(matches!(
        repository
            .load_snapshot(session_key, first.snapshot_key)
            .expect_err("swapped checkpoint"),
        SessionError::MetadataMismatch(_)
    ));
    fs::write(&first_checkpoint, original).expect("restore first checkpoint");

    let first_directory = snapshot_directory(&root, session_key, first.snapshot_key);
    let second_directory = snapshot_directory(&root, session_key, second.snapshot_key);
    fs::remove_dir_all(&first_directory).expect("remove first snapshot directory");
    fs::rename(&second_directory, &first_directory).expect("swap whole snapshot directory");
    assert!(matches!(
        repository
            .load_snapshot(session_key, first.snapshot_key)
            .expect_err("swapped snapshot directory"),
        SessionError::IdentityMismatch(_)
    ));

    let before = tree(&root);
    assert!(matches!(
        repository
            .load_snapshot(999, 1)
            .expect_err("unknown session"),
        SessionError::UnknownStorageSession(999)
    ));
    assert!(matches!(
        repository
            .load_snapshot(session_key, 999)
            .expect_err("unknown snapshot"),
        SessionError::UnknownStorageSnapshot { .. }
    ));
    assert!(matches!(
        repository
            .delete_snapshot(session_key, 999)
            .expect_err("delete unknown snapshot"),
        SessionError::UnknownStorageSnapshot { .. }
    ));
    assert!(matches!(
        repository
            .delete_session(999)
            .expect_err("delete unknown session"),
        SessionError::UnknownStorageSession(999)
    ));
    assert_eq!(
        tree(&root),
        before,
        "unknown-key failures must not create or delete storage state"
    );
}

#[test]
fn arbitrary_record_bytes_never_panic() {
    let (root, repository, session_key, entry) = setup("arbitrary");
    let directory = snapshot_directory(&root, session_key, entry.snapshot_key);
    let session_metadata =
        fs::read(session_metadata_path(&root, session_key)).expect("read session metadata");
    let snapshot_metadata = fs::read(directory.join("snapshot.cfmeta")).expect("read metadata");
    let checkpoint = fs::read(directory.join("checkpoint.cfsnap")).expect("read checkpoint");

    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for length in 0..64 {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push((state >> 24).to_le_bytes()[0]);
        }

        fs::write(directory.join("checkpoint.cfsnap"), &bytes).expect("write fuzz checkpoint");
        assert!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .is_err()
        );
        assert!(repository.list_snapshots(session_key).is_ok());
        fs::write(directory.join("checkpoint.cfsnap"), &checkpoint).expect("restore checkpoint");

        fs::write(directory.join("snapshot.cfmeta"), &bytes).expect("write fuzz metadata");
        assert!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .is_err()
        );
        assert!(repository.list_snapshots(session_key).is_ok());
        fs::write(directory.join("snapshot.cfmeta"), &snapshot_metadata)
            .expect("restore snapshot metadata");

        fs::write(session_metadata_path(&root, session_key), &bytes)
            .expect("write fuzz session metadata");
        assert!(
            repository
                .load_snapshot(session_key, entry.snapshot_key)
                .is_err()
        );
        assert!(repository.list_sessions().is_ok());
        fs::write(session_metadata_path(&root, session_key), &session_metadata)
            .expect("restore session metadata");
    }
}
