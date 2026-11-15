//! Durable snapshot repository operations.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chronofork_core::{SessionId, SnapshotId, hash_bytes};

use crate::error::SessionError;
use crate::snapshot::Snapshot;
use crate::storage::lock::{self, LockIdentity};
use crate::storage::metadata::{MetadataRecord, SessionMetadata, SnapshotMetadata};
use crate::storage::root::{
    self, CHECKPOINT_FILE, LAYOUT_DIR, SESSION_METADATA_FILE, SNAPSHOT_METADATA_FILE,
    SNAPSHOTS_DIR, STORE_MARKER_FILE, StorageRoot, TransientKind,
};
use crate::storage::{
    DurableLineage, LoadedSnapshot, ReconciliationOutcome, SessionEntry, SnapshotEntry,
    StorageAnomaly, StorageAnomalyKind, StorageLimits, StorageListing, WriterLock,
};

/// A durable repository over an explicit, caller-owned storage root.
///
/// The repository stores canonical [`Snapshot::to_bytes`] envelopes verbatim,
/// validates them through [`Snapshot::from_bytes`] and records non-canonical
/// metadata separately. It performs no automatic garbage collection and never
/// rewrites a published checkpoint.
///
/// Mutating operations acquire the explicit per-session writer lock, whose canonical
/// name is published by atomically hard-linking a fully written private record.
/// Readers take no lock: a published snapshot directory is immutable, so a reader
/// observes a complete old or new object, never a torn checkpoint. The only
/// root-level serialization is the short-lived `state/allocation.lock`, held only
/// while a session key is minted; it is never held during serialization or checkpoint
/// I/O. There is no hidden global lock.
#[derive(Debug)]
pub struct SnapshotRepository {
    root: StorageRoot,
}

impl SnapshotRepository {
    /// Opens an initialized root with the default resource limits.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::open_with_limits(path, StorageLimits::default())
    }

    /// Opens an initialized root with explicit limits.
    pub fn open_with_limits(
        path: impl AsRef<Path>,
        limits: StorageLimits,
    ) -> Result<Self, SessionError> {
        Ok(Self {
            root: StorageRoot::open(path.as_ref(), limits)?,
        })
    }

    /// Initializes an empty directory or opens an already-initialized root.
    pub fn initialize(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::initialize_with_limits(path, StorageLimits::default())
    }

    /// Initializes an empty directory or opens an already-initialized root with
    /// explicit limits.
    pub fn initialize_with_limits(
        path: impl AsRef<Path>,
        limits: StorageLimits,
    ) -> Result<Self, SessionError> {
        Ok(Self {
            root: StorageRoot::initialize(path.as_ref(), limits)?,
        })
    }

    /// Creates a new durable session and publishes its metadata record.
    ///
    /// The durable session key is minted from `state/session-keys` and persisted
    /// before publication, so a failure can leave a gap but never reuses a key.
    pub fn create_session(
        &self,
        recorded_session_id: SessionId,
        name: Option<&str>,
        lineage: DurableLineage,
    ) -> Result<SessionEntry, SessionError> {
        self.root.limits().validate_optional_name(name)?;
        lineage.validate()?;
        let session_key = self.reserve_session_key()?;
        let metadata = SessionMetadata {
            storage_session_key: session_key,
            recorded_session_id: recorded_session_id.raw(),
            name: name.map(str::to_owned),
            parent_session_key: lineage.parent_session_key,
            origin_snapshot_key: lineage.origin_snapshot_key,
            next_snapshot_key: 1,
            next_creation_sequence: 1,
            created_at_unix_ms: root::now_unix_ms(),
        };
        self.publish_session(&metadata)?;
        Ok(metadata.entry())
    }

    /// Acquires the cross-process writer lock for one durable session.
    ///
    /// Acquisition never waits and never steals an expired lease. A second writer
    /// receives [`SessionError::WriterContended`] naming the observed owner; an
    /// empty, partial or otherwise unreadable canonical lock is also contention, and
    /// is never reported as [`SessionError::CorruptLock`] during acquisition. The
    /// returned handle has no hidden release on drop; call
    /// [`Self::release_writer_lock`].
    pub fn acquire_writer_lock(&self, session_key: u64) -> Result<WriterLock, SessionError> {
        self.acquire_writer_lock_with_lease(session_key, lock::DEFAULT_LEASE)
    }

    /// Acquires the writer lock with an explicit advisory lease.
    ///
    /// The lease is diagnostic metadata for explicit recovery only. `Duration::ZERO`
    /// records an already-expired lease, which still blocks acquisition: expiry never
    /// authorizes stealing. A filesystem without hard links fails closed with
    /// [`SessionError::LockPrimitiveUnsupported`].
    pub fn acquire_writer_lock_with_lease(
        &self,
        session_key: u64,
        lease: Duration,
    ) -> Result<WriterLock, SessionError> {
        let directory = self.root.session_dir(session_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSession(session_key));
        }
        let record = lock::acquire(&directory, lock::WRITER_LOCK_FILE, Some(session_key), lease)?;
        Ok(WriterLock::new(session_key, self.root.path(), &record))
    }

    /// Releases a writer lock whose expected owner and generation still match.
    ///
    /// A changed successor returns [`SessionError::NotLockOwner`] and is preserved;
    /// no canonical lock record is deleted.
    pub fn release_writer_lock(&self, lock: &WriterLock) -> Result<(), SessionError> {
        let directory = self.root.session_dir(lock.session_key())?;
        lock::release(
            &directory,
            lock::WRITER_LOCK_FILE,
            Some(lock.session_key()),
            lock.identity(),
        )
    }

    /// Explicitly recovers a writer lock after operator confirmation that no writer
    /// is live.
    ///
    /// The observed owner and generation must match exactly. A non-expired lease
    /// returns [`SessionError::WriterContended`] unless `force` is set; forcing still
    /// requires the owner/generation check and the rename-aside protocol, so a
    /// changed successor returns [`SessionError::NotLockOwner`] and is preserved. An
    /// unreadable record returns [`SessionError::CorruptLock`] and removes nothing.
    pub fn recover_writer_lock(
        &self,
        session_key: u64,
        expected: &LockIdentity,
        force: bool,
    ) -> Result<(), SessionError> {
        let directory = self.root.session_dir(session_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSession(session_key));
        }
        lock::recover(
            &directory,
            lock::WRITER_LOCK_FILE,
            Some(session_key),
            expected,
            force,
        )
    }

    /// Explicitly recovers the root key-allocation lock.
    ///
    /// Recovery clears the lock only; it never changes the persisted session-key
    /// counter. The same owner/generation and force rules as writer-lock recovery
    /// apply.
    pub fn recover_allocation_lock(
        &self,
        expected: &LockIdentity,
        force: bool,
    ) -> Result<(), SessionError> {
        let state = self.root.state_dir();
        if !self.root.require_directory(&state)? {
            return Err(SessionError::NotInitialized);
        }
        lock::recover(&state, lock::ALLOCATION_LOCK_FILE, None, expected, force)
    }

    /// Reconciles an object whose publish or delete returned
    /// [`SessionError::DurabilityUnconfirmed`].
    ///
    /// Reconciliation reacquires the session writer lock, retries the parent-directory
    /// sync and re-validates the visible object. It distinguishes visible-confirmed
    /// from visible-unconfirmed and reports a lost object without resurrecting or
    /// repairing it. It never blindly retries a publish.
    pub fn reconcile_snapshot(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<ReconciliationOutcome, SessionError> {
        let lock = self.acquire_writer_lock(session_key)?;
        let outcome = self.reconcile_snapshot_locked(session_key, snapshot_key);
        let released = self.release_writer_lock(&lock);
        match (outcome, released) {
            (Ok(outcome), Ok(())) => Ok(outcome),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), _) => Err(error),
        }
    }

    fn reconcile_snapshot_locked(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<ReconciliationOutcome, SessionError> {
        match self.load_snapshot(session_key, snapshot_key) {
            Ok(_) => {}
            Err(SessionError::UnknownStorageSnapshot { .. }) => {
                return Ok(ReconciliationOutcome::Lost);
            }
            Err(error) => return Err(error),
        }
        let snapshots = self.root.snapshots_dir(session_key)?;
        match root::sync_dir(&snapshots) {
            Ok(()) => Ok(ReconciliationOutcome::VisibleConfirmed),
            Err(_) => Ok(ReconciliationOutcome::VisibleUnconfirmed),
        }
    }

    /// Loads one durable session record.
    pub fn session(&self, session_key: u64) -> Result<SessionEntry, SessionError> {
        Ok(self.load_session_metadata(session_key)?.entry())
    }

    /// Stores a checkpoint as a new snapshot under an existing durable session.
    ///
    /// The checkpoint file is exactly [`Snapshot::to_bytes`] for a canonical
    /// snapshot whose ID is the durable snapshot key. The input snapshot supplies
    /// the backend state and the diagnostic origin session; its runtime snapshot
    /// ID is not a durable identity.
    pub fn save_snapshot(
        &self,
        session_key: u64,
        snapshot: &Snapshot,
        name: Option<&str>,
    ) -> Result<SnapshotEntry, SessionError> {
        self.with_writer_lock(session_key, |repository, lock| {
            repository.save_snapshot_with_lock(lock, snapshot, name)
        })
    }

    /// Publishes a snapshot while the caller's already-held writer lock is verified.
    ///
    /// This is the lock-aware half of a read-modify-publish workflow: the caller
    /// acquires the per-session writer lock once through [`Self::with_writer_lock`]
    /// (or [`Self::acquire_writer_lock`]), reads the base checkpoint while that lock
    /// is held, and publishes through this method. The handle must come from this
    /// repository's root and the canonical record must still name it; otherwise the
    /// operation fails closed with [`SessionError::NotLockOwner`] and publishes
    /// nothing.
    pub fn save_snapshot_with_lock(
        &self,
        lock: &WriterLock,
        snapshot: &Snapshot,
        name: Option<&str>,
    ) -> Result<SnapshotEntry, SessionError> {
        self.root.limits().validate_optional_name(name)?;
        self.verify_writer_lock(lock)?;
        self.save_snapshot_locked(lock.session_key(), snapshot, name)
    }

    fn save_snapshot_locked(
        &self,
        session_key: u64,
        snapshot: &Snapshot,
        name: Option<&str>,
    ) -> Result<SnapshotEntry, SessionError> {
        let session_metadata = self.load_session_metadata(session_key)?;
        let snapshot_key = session_metadata.next_snapshot_key;
        let creation_sequence = session_metadata.next_creation_sequence;

        let snapshot_directory = self.root.snapshot_dir(session_key, snapshot_key)?;
        if root::entry_metadata(&snapshot_directory)?.is_some() {
            return Err(SessionError::SnapshotExists {
                session: session_key,
                snapshot: snapshot_key,
            });
        }

        let stored = Snapshot::new(
            SnapshotId::from_raw(snapshot_key),
            snapshot.origin_session(),
            snapshot.backend().clone(),
        );
        let checkpoint = stored.to_bytes()?;
        let checkpoint_length =
            u64::try_from(checkpoint.len()).map_err(|_| SessionError::FileTooLarge {
                path: snapshot_directory.join(CHECKPOINT_FILE),
                limit: self.root.limits().max_checkpoint_bytes,
                found: u64::MAX,
            })?;
        if checkpoint_length > self.root.limits().max_checkpoint_bytes {
            return Err(SessionError::FileTooLarge {
                path: snapshot_directory.join(CHECKPOINT_FILE),
                limit: self.root.limits().max_checkpoint_bytes,
                found: checkpoint_length,
            });
        }

        let checkpoint_sha256 = hash_bytes(&checkpoint);
        let metadata = SnapshotMetadata {
            storage_session_key: session_key,
            storage_snapshot_key: snapshot_key,
            recorded_snapshot_id: snapshot_key,
            recorded_session_id: stored.origin_session().raw(),
            name: name.map(str::to_owned),
            backend_id: stored.backend().backend_id().as_str().to_owned(),
            backend_version: stored.backend().backend_version().to_owned(),
            backend_format_version: stored.backend().format_version(),
            tick: stored.tick().raw(),
            config_hash: *stored.backend().config_hash().as_bytes(),
            state_hash: *stored.backend().state_hash().as_bytes(),
            checkpoint_length,
            checkpoint_sha256: *checkpoint_sha256.as_bytes(),
            parent_session_key: session_metadata.parent_session_key,
            origin_snapshot_key: session_metadata.origin_snapshot_key,
            creation_sequence,
            created_at_unix_ms: root::now_unix_ms(),
        };
        self.check_metadata_size(
            &self
                .root
                .snapshot_metadata_path(session_key, snapshot_key)?,
            MetadataRecord::Snapshot(metadata.clone()).encode()?.len(),
        )?;

        let mut reserved = session_metadata.clone();
        reserved.next_snapshot_key = snapshot_key
            .checked_add(1)
            .ok_or(SessionError::IdSpaceExhausted)?;
        reserved.next_creation_sequence = creation_sequence
            .checked_add(1)
            .ok_or(SessionError::IdSpaceExhausted)?;
        self.replace_session_metadata(session_key, &reserved)?;

        self.publish_snapshot(&metadata, &checkpoint)?;
        Ok(metadata.entry())
    }

    /// Loads and fully validates one snapshot.
    pub fn load_snapshot(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<LoadedSnapshot, SessionError> {
        let (snapshot, metadata, _) = self.load_snapshot_parts(session_key, snapshot_key)?;
        Ok(LoadedSnapshot {
            snapshot,
            entry: metadata.entry(),
        })
    }

    /// Lists durable sessions by session key.
    pub fn list_sessions(&self) -> Result<StorageListing<SessionEntry>, SessionError> {
        let directory = self.root.sessions_dir();
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::NotInitialized);
        }
        let mut entries = Vec::new();
        let mut anomalies = Vec::new();
        let mut seen = 0_usize;
        for entry in read_directory(self.root.path())? {
            let entry = entry
                .map_err(|error| root::io_error("read storage root", self.root.path(), error))?;
            let name = match entry.file_name().to_str() {
                Some(name) => name.to_owned(),
                None => {
                    seen += 1;
                    self.check_list_cap(seen)?;
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::UnexpectedEntry));
                    continue;
                }
            };
            if name == STORE_MARKER_FILE || name == LAYOUT_DIR {
                continue;
            }
            seen += 1;
            self.check_list_cap(seen)?;
            if let Some(kind) = root::transient_kind(&name) {
                anomalies.push(anomaly(entry.path(), transient_anomaly(kind)));
                continue;
            }
            match root::parse_layout_name(&name) {
                Some(version) => {
                    anomalies.push(anomaly(
                        entry.path(),
                        StorageAnomalyKind::FutureLayoutVersion(version),
                    ));
                }
                None => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::UnexpectedEntry));
                }
            }
        }
        for entry in read_directory(&directory)? {
            seen += 1;
            self.check_list_cap(seen)?;
            let entry =
                entry.map_err(|error| root::io_error("read directory entry", &directory, error))?;
            let name = match entry.file_name().to_str() {
                Some(name) => name.to_owned(),
                None => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::UnexpectedEntry));
                    continue;
                }
            };
            if let Some(kind) = root::transient_kind(&name) {
                anomalies.push(anomaly(entry.path(), transient_anomaly(kind)));
                continue;
            }
            let session_key = match root::parse_keyed_name(&name, "session-") {
                Ok(key) => key,
                Err(_) => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::InvalidKey));
                    continue;
                }
            };
            let session_directory = entry.path();
            let is_directory = entry
                .file_type()
                .map_err(|error| {
                    root::io_error("inspect directory entry", &session_directory, error)
                })?
                .is_dir();
            if is_directory {
                self.collect_private_lock_anomalies(&session_directory, &mut anomalies, &mut seen)?;
            }
            match self.load_session_metadata(session_key) {
                Ok(metadata) => entries.push(metadata.entry()),
                Err(error) => {
                    anomalies.push(anomaly(
                        entry.path(),
                        StorageAnomalyKind::CorruptObject(error),
                    ));
                }
            }
        }
        let state = self.root.state_dir();
        if self.root.require_directory(&state)? {
            self.collect_private_lock_anomalies(&state, &mut anomalies, &mut seen)?;
        }
        entries.sort_by_key(|entry| entry.session_key);
        Ok(StorageListing::new(entries, anomalies))
    }

    /// Reports private lock-protocol leftovers as typed diagnostics.
    ///
    /// Only exact canonical lock names are locks, so every dotted `writer.lock.*` or
    /// `allocation.lock.*` record is diagnosable and is never loaded or reclaimed.
    fn collect_private_lock_anomalies(
        &self,
        directory: &Path,
        anomalies: &mut Vec<StorageAnomaly>,
        seen: &mut usize,
    ) -> Result<(), SessionError> {
        for entry in read_directory(directory)? {
            *seen += 1;
            self.check_list_cap(*seen)?;
            let entry =
                entry.map_err(|error| root::io_error("read directory entry", directory, error))?;
            let name = match entry.file_name().to_str() {
                Some(name) => name.to_owned(),
                None => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::UnexpectedEntry));
                    continue;
                }
            };
            if lock::is_private_lock_record(&name) {
                anomalies.push(anomaly(
                    entry.path(),
                    StorageAnomalyKind::LockProtocolRecord,
                ));
            }
        }
        Ok(())
    }

    /// Lists validated snapshots for one session, ordered by logical creation
    /// sequence and then snapshot key.
    pub fn list_snapshots(
        &self,
        session_key: u64,
    ) -> Result<StorageListing<SnapshotEntry>, SessionError> {
        self.load_session_metadata(session_key)?;
        let directory = self.root.snapshots_dir(session_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::MissingRecord(directory));
        }
        let mut entries = Vec::new();
        let mut anomalies = Vec::new();
        let mut seen = 0_usize;
        for entry in read_directory(&directory)? {
            seen += 1;
            self.check_list_cap(seen)?;
            let entry =
                entry.map_err(|error| root::io_error("read directory entry", &directory, error))?;
            let name = match entry.file_name().to_str() {
                Some(name) => name.to_owned(),
                None => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::UnexpectedEntry));
                    continue;
                }
            };
            if let Some(kind) = root::transient_kind(&name) {
                anomalies.push(anomaly(entry.path(), transient_anomaly(kind)));
                continue;
            }
            let snapshot_key = match root::parse_keyed_name(&name, "snapshot-") {
                Ok(key) => key,
                Err(_) => {
                    anomalies.push(anomaly(entry.path(), StorageAnomalyKind::InvalidKey));
                    continue;
                }
            };
            match self.load_snapshot(session_key, snapshot_key) {
                Ok(loaded) => entries.push(loaded.entry),
                Err(error) => {
                    anomalies.push(anomaly(
                        entry.path(),
                        StorageAnomalyKind::CorruptObject(error),
                    ));
                }
            }
        }
        entries.sort_by_key(|entry| (entry.creation_sequence, entry.snapshot_key));
        Ok(StorageListing::new(entries, anomalies))
    }

    /// Returns the latest validated snapshot by logical creation sequence.
    pub fn latest_snapshot(&self, session_key: u64) -> Result<Option<SnapshotEntry>, SessionError> {
        Ok(self.list_snapshots(session_key)?.entries.pop())
    }

    /// Rewrites only a snapshot's metadata record; the checkpoint is untouched.
    pub fn rename_snapshot(
        &self,
        session_key: u64,
        snapshot_key: u64,
        name: Option<&str>,
    ) -> Result<SnapshotEntry, SessionError> {
        self.root.limits().validate_optional_name(name)?;
        self.with_writer_lock(session_key, |repository, _lock| {
            repository.rename_snapshot_locked(session_key, snapshot_key, name)
        })
    }

    fn rename_snapshot_locked(
        &self,
        session_key: u64,
        snapshot_key: u64,
        name: Option<&str>,
    ) -> Result<SnapshotEntry, SessionError> {
        let (_, mut metadata, _) = self.load_snapshot_parts(session_key, snapshot_key)?;
        metadata.name = name.map(str::to_owned);
        let bytes = MetadataRecord::Snapshot(metadata.clone()).encode()?;
        let directory = self.root.snapshot_dir(session_key, snapshot_key)?;
        let path = self
            .root
            .snapshot_metadata_path(session_key, snapshot_key)?;
        root::replace_file_atomic(
            &directory,
            &path,
            "snapshot.cfmeta.tmp",
            &bytes,
            "replace snapshot metadata",
        )?;
        Ok(metadata.entry())
    }

    /// Explicitly deletes one snapshot by renaming it to a tombstone and
    /// removing the tombstone. It never cascades and never runs automatically.
    pub fn delete_snapshot(&self, session_key: u64, snapshot_key: u64) -> Result<(), SessionError> {
        self.with_writer_lock(session_key, |repository, _lock| {
            repository.delete_snapshot_locked(session_key, snapshot_key)
        })
    }

    fn delete_snapshot_locked(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<(), SessionError> {
        self.load_session_metadata(session_key)?;
        let snapshots = self.root.snapshots_dir(session_key)?;
        if !self.root.require_directory(&snapshots)? {
            return Err(SessionError::UnknownStorageSnapshot {
                session: session_key,
                snapshot: snapshot_key,
            });
        }
        let directory = self.root.snapshot_dir(session_key, snapshot_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSnapshot {
                session: session_key,
                snapshot: snapshot_key,
            });
        }
        let trash = self.root.trash_dir(&snapshots, "snapshot", snapshot_key)?;
        fs::rename(&directory, &trash)
            .map_err(|error| root::io_error("delete snapshot", &directory, error))?;
        root::sync_dir(&snapshots).map_err(|_| SessionError::DurabilityUnconfirmed {
            operation: "delete snapshot",
            path: snapshots,
        })?;
        root::remove_transient(&trash);
        Ok(())
    }

    /// Explicitly deletes an empty session directory.
    ///
    /// A session that still contains snapshot directories is rejected; deletion
    /// never cascades.
    pub fn delete_session(&self, session_key: u64) -> Result<(), SessionError> {
        let lock = self.acquire_writer_lock(session_key)?;
        let trash = match self.stage_session_deletion(session_key) {
            Ok(trash) => trash,
            Err(error) => return Err(self.release_after_failure(&lock, error)),
        };
        // The session-directory rename also removes the canonical writer-lock name
        // from service, so a separate release is neither possible nor needed after
        // this commit point. On a post-rename sync failure the tombstone keeps the
        // record as a diagnostic and is never treated as a live lock.
        let sessions = self.root.sessions_dir();
        match root::sync_dir(&sessions) {
            Ok(()) => {
                root::remove_transient(&trash);
                Ok(())
            }
            Err(_) => Err(SessionError::DurabilityUnconfirmed {
                operation: "delete session",
                path: sessions,
            }),
        }
    }

    fn stage_session_deletion(&self, session_key: u64) -> Result<PathBuf, SessionError> {
        let directory = self.root.session_dir(session_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSession(session_key));
        }
        let snapshots = self.root.snapshots_dir(session_key)?;
        if self.root.require_directory(&snapshots)? {
            let mut entries = read_directory(&snapshots)?;
            if entries.next().is_some() {
                return Err(SessionError::SessionNotEmpty(session_key));
            }
        }
        let sessions = self.root.sessions_dir();
        if !self.root.require_directory(&sessions)? {
            return Err(SessionError::NotInitialized);
        }
        let trash = self.root.trash_dir(&sessions, "session", session_key)?;
        fs::rename(&directory, &trash)
            .map_err(|error| root::io_error("delete session", &directory, error))?;
        Ok(trash)
    }

    fn release_after_failure(&self, lock: &WriterLock, operation: SessionError) -> SessionError {
        match self.release_writer_lock(lock) {
            Ok(()) | Err(SessionError::NotLockOwner { .. }) => operation,
            Err(error) => error,
        }
    }

    fn load_session_metadata(&self, session_key: u64) -> Result<SessionMetadata, SessionError> {
        let directory = self.root.session_dir(session_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSession(session_key));
        }
        let path = self.root.session_metadata_path(session_key)?;
        if !self.root.require_regular_file(&path)? {
            return Err(SessionError::MissingRecord(path));
        }
        let bytes = root::read_bounded_file(
            &path,
            self.root.limits().max_metadata_bytes,
            "read session metadata",
        )?;
        let record = MetadataRecord::decode(&bytes, self.root.limits().max_name_bytes)?;
        let MetadataRecord::Session(metadata) = record else {
            return Err(SessionError::MetadataMismatch(
                "session.cfmeta contains a snapshot record".to_owned(),
            ));
        };
        metadata.validate_for(session_key)?;
        Ok(metadata)
    }

    fn load_snapshot_parts(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<(Snapshot, SnapshotMetadata, SessionMetadata), SessionError> {
        let session_metadata = self.load_session_metadata(session_key)?;
        let directory = self.root.snapshot_dir(session_key, snapshot_key)?;
        if !self.root.require_directory(&directory)? {
            return Err(SessionError::UnknownStorageSnapshot {
                session: session_key,
                snapshot: snapshot_key,
            });
        }
        let metadata_path = self
            .root
            .snapshot_metadata_path(session_key, snapshot_key)?;
        if !self.root.require_regular_file(&metadata_path)? {
            return Err(SessionError::MissingRecord(metadata_path));
        }
        let metadata_bytes = root::read_bounded_file(
            &metadata_path,
            self.root.limits().max_metadata_bytes,
            "read snapshot metadata",
        )?;
        let record = MetadataRecord::decode(&metadata_bytes, self.root.limits().max_name_bytes)?;
        let MetadataRecord::Snapshot(metadata) = record else {
            return Err(SessionError::MetadataMismatch(
                "snapshot.cfmeta contains a session record".to_owned(),
            ));
        };

        let checkpoint_path = self.root.checkpoint_path(session_key, snapshot_key)?;
        if !self.root.require_regular_file(&checkpoint_path)? {
            return Err(SessionError::MissingRecord(checkpoint_path));
        }
        let checkpoint = root::read_bounded_file(
            &checkpoint_path,
            self.root.limits().max_checkpoint_bytes,
            "read checkpoint",
        )?;
        let checkpoint_length =
            u64::try_from(checkpoint.len()).map_err(|_| SessionError::FileTooLarge {
                path: checkpoint_path.clone(),
                limit: self.root.limits().max_checkpoint_bytes,
                found: u64::MAX,
            })?;
        if checkpoint_length != metadata.checkpoint_length {
            return Err(SessionError::MetadataMismatch(format!(
                "checkpoint is {checkpoint_length} bytes, metadata records {}",
                metadata.checkpoint_length
            )));
        }
        let checkpoint_sha256 = hash_bytes(&checkpoint);
        if checkpoint_sha256.as_bytes() != &metadata.checkpoint_sha256 {
            return Err(SessionError::MetadataMismatch(
                "checkpoint SHA-256 does not match metadata".to_owned(),
            ));
        }

        let snapshot = Snapshot::from_bytes(&checkpoint)?;
        metadata.validate_against(&snapshot, session_key, snapshot_key)?;
        if metadata.parent_session_key != session_metadata.parent_session_key
            || metadata.origin_snapshot_key != session_metadata.origin_snapshot_key
        {
            return Err(SessionError::MetadataMismatch(
                "snapshot lineage does not match the session record".to_owned(),
            ));
        }
        Ok((snapshot, metadata, session_metadata))
    }

    fn publish_session(&self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        let sessions = self.root.sessions_dir();
        if !self.root.require_directory(&sessions)? {
            return Err(SessionError::NotInitialized);
        }
        let final_directory = self.root.session_dir(metadata.storage_session_key)?;
        if root::entry_metadata(&final_directory)?.is_some() {
            return Err(SessionError::SessionConflict(metadata.storage_session_key));
        }
        let staging = self
            .root
            .staging_dir(&sessions, "session", metadata.storage_session_key)?;
        let result = (|| {
            fs::create_dir(staging.join(SNAPSHOTS_DIR))
                .map_err(|error| root::io_error("create snapshots directory", &staging, error))?;
            let bytes = MetadataRecord::Session(metadata.clone()).encode()?;
            self.check_metadata_size(&staging.join(SESSION_METADATA_FILE), bytes.len())?;
            write_staged_file(
                &staging,
                "session.cfmeta.tmp",
                SESSION_METADATA_FILE,
                &bytes,
                "write session metadata",
            )?;
            fs::rename(&staging, &final_directory).map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    SessionError::SessionConflict(metadata.storage_session_key)
                } else {
                    root::io_error("publish session", &final_directory, error)
                }
            })?;
            root::sync_dir(&sessions).map_err(|_| SessionError::DurabilityUnconfirmed {
                operation: "publish session",
                path: sessions.clone(),
            })
        })();
        if result.is_err() {
            root::remove_transient(&staging);
        }
        result
    }

    fn publish_snapshot(
        &self,
        metadata: &SnapshotMetadata,
        checkpoint: &[u8],
    ) -> Result<(), SessionError> {
        let snapshots = self.root.snapshots_dir(metadata.storage_session_key)?;
        if !self.root.require_directory(&snapshots)? {
            return Err(SessionError::UnknownStorageSession(
                metadata.storage_session_key,
            ));
        }
        let final_directory = self
            .root
            .snapshot_dir(metadata.storage_session_key, metadata.storage_snapshot_key)?;
        if root::entry_metadata(&final_directory)?.is_some() {
            return Err(SessionError::SnapshotExists {
                session: metadata.storage_session_key,
                snapshot: metadata.storage_snapshot_key,
            });
        }
        let staging =
            self.root
                .staging_dir(&snapshots, "snapshot", metadata.storage_snapshot_key)?;
        let result = (|| {
            write_staged_file(
                &staging,
                "checkpoint.cfsnap.tmp",
                CHECKPOINT_FILE,
                checkpoint,
                "write checkpoint",
            )?;
            let metadata_bytes = MetadataRecord::Snapshot(metadata.clone()).encode()?;
            write_staged_file(
                &staging,
                "snapshot.cfmeta.tmp",
                SNAPSHOT_METADATA_FILE,
                &metadata_bytes,
                "write snapshot metadata",
            )?;
            fs::rename(&staging, &final_directory).map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    SessionError::SnapshotExists {
                        session: metadata.storage_session_key,
                        snapshot: metadata.storage_snapshot_key,
                    }
                } else {
                    root::io_error("publish snapshot", &final_directory, error)
                }
            })?;
            root::sync_dir(&snapshots).map_err(|_| SessionError::DurabilityUnconfirmed {
                operation: "publish snapshot",
                path: snapshots.clone(),
            })
        })();
        if result.is_err() {
            root::remove_transient(&staging);
        }
        result
    }

    fn replace_session_metadata(
        &self,
        session_key: u64,
        metadata: &SessionMetadata,
    ) -> Result<(), SessionError> {
        let directory = self.root.session_dir(session_key)?;
        let path = self.root.session_metadata_path(session_key)?;
        let bytes = MetadataRecord::Session(metadata.clone()).encode()?;
        self.check_metadata_size(&path, bytes.len())?;
        root::replace_file_atomic(
            &directory,
            &path,
            "session.cfmeta.tmp",
            &bytes,
            "replace session metadata",
        )
    }

    /// Runs a storage workflow under one held session writer lock.
    ///
    /// This is the supported way to make a read-modify-publish workflow linearizable:
    /// the operation acquires nothing itself, reads through ordinary lock-free
    /// methods while the lock is held, and publishes through
    /// [`Self::save_snapshot_with_lock`]. A second process fails at acquisition with
    /// [`SessionError::WriterContended`] before it can read a stale base.
    ///
    /// The lock is released before the result is returned, on every success and error
    /// path, including [`SessionError::DurabilityUnconfirmed`]. If both the operation
    /// and the release fail, the operation error is the root cause.
    pub fn with_writer_lock<T, E>(
        &self,
        session_key: u64,
        operation: impl FnOnce(&Self, &WriterLock) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<SessionError>,
    {
        let lock = self.acquire_writer_lock(session_key).map_err(E::from)?;
        let result = operation(self, &lock);
        let released = self.release_writer_lock(&lock).map_err(E::from);
        match (result, released) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), _) => Err(error),
        }
    }

    /// Fails closed unless `lock` belongs to this root and still owns the canonical
    /// per-session record.
    fn verify_writer_lock(&self, lock: &WriterLock) -> Result<(), SessionError> {
        if lock.store_root() != self.root.path() {
            return Err(SessionError::NotLockOwner {
                session: Some(lock.session_key()),
                expected: lock.identity().clone(),
            });
        }
        let directory = self.root.session_dir(lock.session_key())?;
        lock::verify_held(
            &directory,
            lock::WRITER_LOCK_FILE,
            Some(lock.session_key()),
            lock.identity(),
        )
    }

    /// Mints and persists the next durable session key under the root allocation
    /// lock.
    ///
    /// The lock is held only for the read-modify-write of `state/session-keys`,
    /// including the atomic counter update and its parent sync. It is never held
    /// during session publication, serialization or checkpoint I/O. A persisted
    /// counter that advanced past a key makes that key a permanent gap.
    fn reserve_session_key(&self) -> Result<u64, SessionError> {
        let state = self.root.state_dir();
        if !self.root.require_directory(&state)? {
            return Err(SessionError::NotInitialized);
        }
        let allocation = lock::acquire(
            &state,
            lock::ALLOCATION_LOCK_FILE,
            None,
            lock::DEFAULT_LEASE,
        )?;
        let reserved: Result<u64, SessionError> = (|| {
            let key = self.root.read_session_keys()?;
            let next = key.checked_add(1).ok_or(SessionError::IdSpaceExhausted)?;
            self.root.write_session_keys(next)?;
            Ok(key)
        })();
        let released = lock::release(
            &state,
            lock::ALLOCATION_LOCK_FILE,
            None,
            &allocation.identity,
        );
        match (reserved, released) {
            (Ok(key), Ok(())) => Ok(key),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), _) => Err(error),
        }
    }

    fn check_metadata_size(&self, path: &Path, length: usize) -> Result<(), SessionError> {
        let found = u64::try_from(length).unwrap_or(u64::MAX);
        if found > self.root.limits().max_metadata_bytes {
            return Err(SessionError::FileTooLarge {
                path: path.to_path_buf(),
                limit: self.root.limits().max_metadata_bytes,
                found,
            });
        }
        Ok(())
    }

    fn check_list_cap(&self, seen: usize) -> Result<(), SessionError> {
        if seen > self.root.limits().max_list_entries {
            return Err(SessionError::ListLimitExceeded {
                limit: self.root.limits().max_list_entries,
            });
        }
        Ok(())
    }
}

fn write_staged_file(
    directory: &Path,
    temporary_name: &str,
    final_name: &str,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), SessionError> {
    let temporary = directory.join(temporary_name);
    let final_path = directory.join(final_name);
    root::write_new_file_synced(&temporary, bytes, operation)?;
    fs::rename(&temporary, &final_path)
        .map_err(|error| root::io_error(operation, &final_path, error))?;
    root::sync_dir(directory)
        .map_err(|error| root::io_error("sync staging directory", directory, error))
}

fn read_directory(path: &Path) -> Result<fs::ReadDir, SessionError> {
    fs::read_dir(path).map_err(|error| root::io_error("read storage directory", path, error))
}

fn anomaly(path: PathBuf, kind: StorageAnomalyKind) -> StorageAnomaly {
    StorageAnomaly { path, kind }
}

fn transient_anomaly(kind: TransientKind) -> StorageAnomalyKind {
    match kind {
        TransientKind::Staging => StorageAnomalyKind::IncompletePublish,
        TransientKind::Trash => StorageAnomalyKind::PendingDeletion,
    }
}
