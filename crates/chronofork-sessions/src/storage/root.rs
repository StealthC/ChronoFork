//! Storage root, versioned layout, confinement and bounded file primitives.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::SessionError;
use crate::storage::{STORAGE_LAYOUT_VERSION, StorageLimits};

pub(crate) const STORE_MARKER_FILE: &str = "chronofork.store";
pub(crate) const STORE_MAGIC: [u8; 8] = *b"CFSTORE1";
pub(crate) const SESSION_KEYS_FILE: &str = "session-keys";
pub(crate) const SESSION_METADATA_FILE: &str = "session.cfmeta";
pub(crate) const SNAPSHOT_METADATA_FILE: &str = "snapshot.cfmeta";
pub(crate) const CHECKPOINT_FILE: &str = "checkpoint.cfsnap";

pub(crate) const LAYOUT_DIR: &str = "v1";
const STATE_DIR: &str = "state";
const SESSIONS_DIR: &str = "sessions";
pub(crate) const SNAPSHOTS_DIR: &str = "snapshots";
const MARKER_LEN: usize = STORE_MAGIC.len() + 2;
const SESSION_KEYS_LEN: usize = 8;
const STAGING_PREFIX: &str = ".staging-";
const TRASH_PREFIX: &str = ".trash-";

/// A transient directory left by an interrupted operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransientKind {
    Staging,
    Trash,
}

/// A validated, canonicalized storage root with its configured limits.
#[derive(Debug)]
pub(crate) struct StorageRoot {
    root: PathBuf,
    limits: StorageLimits,
}

impl StorageRoot {
    /// Opens an initialized root without creating or repairing anything.
    pub(crate) fn open(path: &Path, limits: StorageLimits) -> Result<Self, SessionError> {
        let root = fs::canonicalize(path).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                SessionError::NotInitialized
            } else {
                io_error("open storage root", path, error)
            }
        })?;
        let metadata =
            fs::metadata(&root).map_err(|error| io_error("inspect storage root", &root, error))?;
        if !metadata.is_dir() {
            return Err(SessionError::WrongFileType(root));
        }

        let marker = root.join(STORE_MARKER_FILE);
        match entry_metadata(&marker)? {
            None => return Err(SessionError::NotInitialized),
            Some(metadata) if metadata.file_type().is_symlink() => {
                return Err(SessionError::SymlinkRejected(marker));
            }
            Some(metadata) if !metadata.is_file() => {
                return Err(SessionError::WrongFileType(marker));
            }
            Some(_) => {}
        }
        let marker_bytes = read_bounded_file(
            &marker,
            u64::try_from(MARKER_LEN).unwrap_or(u64::MAX),
            "read storage marker",
        )?;
        parse_marker(&marker_bytes)?;

        let storage = Self { root, limits };
        for directory in [
            storage.layout_dir(),
            storage.state_dir(),
            storage.sessions_dir(),
        ] {
            if !require_directory(&directory)? {
                return Err(SessionError::NotInitialized);
            }
        }
        Ok(storage)
    }

    /// Initializes an empty directory or opens an already-initialized root.
    pub(crate) fn initialize(path: &Path, limits: StorageLimits) -> Result<Self, SessionError> {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(SessionError::WrongFileType(path.to_path_buf())),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir(path)
                    .map_err(|error| io_error("create storage root", path, error))?;
            }
            Err(error) => return Err(io_error("inspect storage root", path, error)),
        }
        let root = fs::canonicalize(path)
            .map_err(|error| io_error("canonicalize storage root", path, error))?;

        if entry_metadata(&root.join(STORE_MARKER_FILE))?.is_some() {
            return Self::open(&root, limits);
        }

        let mut entries =
            fs::read_dir(&root).map_err(|error| io_error("read storage root", &root, error))?;
        if entries.next().is_some() {
            return Err(SessionError::RootNotEmpty(root));
        }

        let storage = Self { root, limits };
        fs::create_dir(storage.layout_dir())
            .map_err(|error| io_error("create layout directory", &storage.layout_dir(), error))?;
        fs::create_dir(storage.state_dir())
            .map_err(|error| io_error("create state directory", &storage.state_dir(), error))?;
        fs::create_dir(storage.sessions_dir()).map_err(|error| {
            io_error("create sessions directory", &storage.sessions_dir(), error)
        })?;
        write_new_file_synced(
            &storage.session_keys_path(),
            &1_u64.to_le_bytes(),
            "initialize session-key counter",
        )?;
        sync_dir(&storage.state_dir())
            .map_err(|error| io_error("sync state directory", &storage.state_dir(), error))?;
        sync_dir(&storage.sessions_dir())
            .map_err(|error| io_error("sync sessions directory", &storage.sessions_dir(), error))?;
        sync_dir(&storage.layout_dir())
            .map_err(|error| io_error("sync layout directory", &storage.layout_dir(), error))?;
        write_new_file_synced(
            &storage.marker_path(),
            &marker_bytes(),
            "create storage marker",
        )?;
        sync_dir(&storage.root)
            .map_err(|error| io_error("sync storage root", &storage.root, error))?;
        Ok(storage)
    }

    pub(crate) fn limits(&self) -> &StorageLimits {
        &self.limits
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    fn marker_path(&self) -> PathBuf {
        self.root.join(STORE_MARKER_FILE)
    }

    fn layout_dir(&self) -> PathBuf {
        self.root.join(LAYOUT_DIR)
    }

    pub(crate) fn state_dir(&self) -> PathBuf {
        self.layout_dir().join(STATE_DIR)
    }

    pub(crate) fn sessions_dir(&self) -> PathBuf {
        self.layout_dir().join(SESSIONS_DIR)
    }

    fn session_keys_path(&self) -> PathBuf {
        self.state_dir().join(SESSION_KEYS_FILE)
    }

    pub(crate) fn session_dir(&self, session_key: u64) -> Result<PathBuf, SessionError> {
        self.confine(self.sessions_dir().join(format!("session-{session_key}")))
    }

    pub(crate) fn session_metadata_path(&self, session_key: u64) -> Result<PathBuf, SessionError> {
        Ok(self.session_dir(session_key)?.join(SESSION_METADATA_FILE))
    }

    pub(crate) fn snapshots_dir(&self, session_key: u64) -> Result<PathBuf, SessionError> {
        self.confine(self.session_dir(session_key)?.join(SNAPSHOTS_DIR))
    }

    pub(crate) fn snapshot_dir(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<PathBuf, SessionError> {
        self.confine(
            self.snapshots_dir(session_key)?
                .join(format!("snapshot-{snapshot_key}")),
        )
    }

    pub(crate) fn snapshot_metadata_path(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<PathBuf, SessionError> {
        Ok(self
            .snapshot_dir(session_key, snapshot_key)?
            .join(SNAPSHOT_METADATA_FILE))
    }

    pub(crate) fn checkpoint_path(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<PathBuf, SessionError> {
        Ok(self
            .snapshot_dir(session_key, snapshot_key)?
            .join(CHECKPOINT_FILE))
    }

    pub(crate) fn staging_dir(
        &self,
        parent: &Path,
        kind: &str,
        key: u64,
    ) -> Result<PathBuf, SessionError> {
        self.confine(create_unique_dir(
            parent,
            &format!("{STAGING_PREFIX}{kind}-{key}"),
        )?)
    }

    pub(crate) fn trash_dir(
        &self,
        parent: &Path,
        kind: &str,
        key: u64,
    ) -> Result<PathBuf, SessionError> {
        self.confine(unique_transient_path(
            parent,
            &format!("{TRASH_PREFIX}{kind}-{key}"),
        )?)
    }

    /// Reads the persisted next session key.
    pub(crate) fn read_session_keys(&self) -> Result<u64, SessionError> {
        let path = self.session_keys_path();
        if !self.require_regular_file(&path).unwrap_or(false) {
            return Err(SessionError::CorruptAllocationCounter(
                "state/session-keys is missing or unreadable".to_owned(),
            ));
        }
        let metadata = match entry_metadata(&path) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                return Err(SessionError::CorruptAllocationCounter(
                    "state/session-keys is missing".to_owned(),
                ));
            }
            Err(_) => {
                return Err(SessionError::CorruptAllocationCounter(
                    "state/session-keys is unreadable".to_owned(),
                ));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SessionError::CorruptAllocationCounter(
                "state/session-keys is not a regular file".to_owned(),
            ));
        }
        if metadata.len() != u64::try_from(SESSION_KEYS_LEN).unwrap_or(u64::MAX) {
            return Err(SessionError::CorruptAllocationCounter(format!(
                "state/session-keys is {} bytes, expected {SESSION_KEYS_LEN}",
                metadata.len()
            )));
        }
        let bytes = match read_bounded_file(
            &path,
            u64::try_from(SESSION_KEYS_LEN).unwrap_or(u64::MAX),
            "read session-key counter",
        ) {
            Ok(bytes) => bytes,
            Err(_) => {
                return Err(SessionError::CorruptAllocationCounter(
                    "state/session-keys is unreadable".to_owned(),
                ));
            }
        };
        if bytes.len() != SESSION_KEYS_LEN {
            return Err(SessionError::CorruptAllocationCounter(format!(
                "state/session-keys is {} bytes, expected {SESSION_KEYS_LEN}",
                bytes.len()
            )));
        }
        let mut raw = [0_u8; SESSION_KEYS_LEN];
        raw.copy_from_slice(&bytes);
        let next = u64::from_le_bytes(raw);
        if next == 0 {
            return Err(SessionError::CorruptAllocationCounter(
                "state/session-keys is zero; keys start at one".to_owned(),
            ));
        }
        Ok(next)
    }

    /// Persists the next durable session key.
    ///
    /// The caller holds the root allocation lock for the read-modify-write; this
    /// helper only replaces the 8-byte counter record atomically.
    pub(crate) fn write_session_keys(&self, next: u64) -> Result<(), SessionError> {
        replace_file_atomic(
            &self.state_dir(),
            &self.session_keys_path(),
            "session-keys.tmp",
            &next.to_le_bytes(),
            "replace session-key counter",
        )
    }

    /// Requires a managed directory, rejecting symlinked ancestors.
    pub(crate) fn require_directory(&self, path: &Path) -> Result<bool, SessionError> {
        self.check_ancestors(path)?;
        require_directory(path)
    }

    /// Requires a managed regular file, rejecting symlinked ancestors.
    pub(crate) fn require_regular_file(&self, path: &Path) -> Result<bool, SessionError> {
        self.check_ancestors(path)?;
        require_regular_file(path)
    }

    fn check_ancestors(&self, path: &Path) -> Result<(), SessionError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| SessionError::PathEscape(path.to_path_buf()))?;
        let mut current = self.root.clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            current.push(component);
            if components.peek().is_none() {
                break;
            }
            if !require_directory(&current)? {
                return Err(SessionError::MissingRecord(current));
            }
        }
        Ok(())
    }

    fn confine(&self, path: PathBuf) -> Result<PathBuf, SessionError> {
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(SessionError::PathEscape(path))
        }
    }
}

/// Encodes the root marker: `CFSTORE1` followed by the little-endian layout version.
pub(crate) fn marker_bytes() -> [u8; MARKER_LEN] {
    let mut bytes = [0_u8; MARKER_LEN];
    bytes[..STORE_MAGIC.len()].copy_from_slice(&STORE_MAGIC);
    bytes[STORE_MAGIC.len()..].copy_from_slice(&STORAGE_LAYOUT_VERSION.to_le_bytes());
    bytes
}

fn parse_marker(bytes: &[u8]) -> Result<(), SessionError> {
    if bytes.len() < STORE_MAGIC.len() || bytes[..STORE_MAGIC.len()] != STORE_MAGIC {
        return Err(SessionError::NotAStorageRoot(
            "marker magic is not CFSTORE1".to_owned(),
        ));
    }
    if bytes.len() != MARKER_LEN {
        return Err(SessionError::NotAStorageRoot(format!(
            "marker is {} bytes, expected {MARKER_LEN}",
            bytes.len()
        )));
    }
    let mut raw = [0_u8; 2];
    raw.copy_from_slice(&bytes[STORE_MAGIC.len()..]);
    let found = u16::from_le_bytes(raw);
    if found != STORAGE_LAYOUT_VERSION {
        return Err(SessionError::UnsupportedLayoutVersion {
            found,
            supported: STORAGE_LAYOUT_VERSION,
        });
    }
    Ok(())
}

/// Inspects a path without following a final symbolic link.
pub(crate) fn entry_metadata(path: &Path) -> Result<Option<Metadata>, SessionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("inspect path", path, error)),
    }
}

/// Requires a directory and rejects symlinks and other file types.
pub(crate) fn require_directory(path: &Path) -> Result<bool, SessionError> {
    match entry_metadata(path)? {
        None => Ok(false),
        Some(metadata) if metadata.file_type().is_symlink() => {
            Err(SessionError::SymlinkRejected(path.to_path_buf()))
        }
        Some(metadata) if metadata.is_dir() => Ok(true),
        Some(_) => Err(SessionError::WrongFileType(path.to_path_buf())),
    }
}

/// Requires a regular file and rejects symlinks and other file types.
pub(crate) fn require_regular_file(path: &Path) -> Result<bool, SessionError> {
    match entry_metadata(path)? {
        None => Ok(false),
        Some(metadata) if metadata.file_type().is_symlink() => {
            Err(SessionError::SymlinkRejected(path.to_path_buf()))
        }
        Some(metadata) if metadata.is_file() => Ok(true),
        Some(_) => Err(SessionError::WrongFileType(path.to_path_buf())),
    }
}

/// Reads a regular file with a length check before allocation and a bounded read.
pub(crate) fn read_bounded_file(
    path: &Path,
    limit: u64,
    operation: &'static str,
) -> Result<Vec<u8>, SessionError> {
    let metadata = match entry_metadata(path)? {
        Some(metadata) => metadata,
        None => {
            return Err(SessionError::Io {
                operation,
                path: path.to_path_buf(),
                kind: ErrorKind::NotFound,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(SessionError::SymlinkRejected(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(SessionError::WrongFileType(path.to_path_buf()));
    }
    if metadata.len() > limit {
        return Err(SessionError::FileTooLarge {
            path: path.to_path_buf(),
            limit,
            found: metadata.len(),
        });
    }
    let file = File::open(path).map_err(|error| io_error(operation, path, error))?;
    read_limited(file, metadata.len(), limit, path)
}

/// Reads at most `limit + 1` bytes so growth after the metadata check is detected.
pub(crate) fn read_limited<R: Read>(
    reader: R,
    declared_len: u64,
    limit: u64,
    path: &Path,
) -> Result<Vec<u8>, SessionError> {
    let capacity = usize::try_from(declared_len).map_err(|_| SessionError::FileTooLarge {
        path: path.to_path_buf(),
        limit,
        found: declared_len,
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    let capped = limit.saturating_add(1);
    reader
        .take(capped)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read file", path, error))?;
    let found = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if found > limit {
        return Err(SessionError::FileTooLarge {
            path: path.to_path_buf(),
            limit,
            found,
        });
    }
    Ok(bytes)
}

/// Writes a new file exclusively and syncs its contents.
pub(crate) fn write_new_file_synced(
    path: &Path,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), SessionError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(operation, path, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error(operation, path, error))?;
    file.sync_all()
        .map_err(|error| io_error(operation, path, error))?;
    Ok(())
}

/// Replaces a published file through a sibling temporary file and a rename.
pub(crate) fn replace_file_atomic(
    directory: &Path,
    final_path: &Path,
    temporary_name: &str,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), SessionError> {
    let temporary = directory.join(temporary_name);
    let _ = fs::remove_file(&temporary);
    if let Err(error) = write_new_file_synced(&temporary, bytes, operation) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, final_path) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(operation, final_path, error));
    }
    sync_dir(directory).map_err(|_| SessionError::DurabilityUnconfirmed {
        operation,
        path: directory.to_path_buf(),
    })
}

/// Syncs a directory entry so a rename survives a crash.
pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

/// Creates a unique transient directory with a bounded, deterministic nonce search.
fn create_unique_dir(parent: &Path, prefix: &str) -> Result<PathBuf, SessionError> {
    for nonce in 0..16_u32 {
        let path = parent.join(format!("{prefix}-{nonce}"));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create staging directory", &path, error)),
        }
    }
    Err(SessionError::Io {
        operation: "create staging directory",
        path: parent.to_path_buf(),
        kind: ErrorKind::AlreadyExists,
    })
}

/// Finds an unused transient path with a bounded, deterministic nonce search.
fn unique_transient_path(parent: &Path, prefix: &str) -> Result<PathBuf, SessionError> {
    for nonce in 0..16_u32 {
        let path = parent.join(format!("{prefix}-{nonce}"));
        if entry_metadata(&path)?.is_none() {
            return Ok(path);
        }
    }
    Err(SessionError::Io {
        operation: "reserve transient name",
        path: parent.to_path_buf(),
        kind: ErrorKind::AlreadyExists,
    })
}

/// Removes a transient tree, ignoring failures; a leftover is a diagnostic.
pub(crate) fn remove_transient(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// Classifies a reserved transient entry name.
pub(crate) fn transient_kind(name: &str) -> Option<TransientKind> {
    if name.starts_with(STAGING_PREFIX) {
        Some(TransientKind::Staging)
    } else if name.starts_with(TRASH_PREFIX) {
        Some(TransientKind::Trash)
    } else {
        None
    }
}

/// Parses a canonical layout directory name such as `v1`.
pub(crate) fn parse_layout_name(name: &str) -> Option<u16> {
    let rest = name.strip_prefix('v')?;
    if rest.is_empty() || rest.starts_with('0') || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// Parses a canonical keyed directory name such as `session-12`.
pub(crate) fn parse_keyed_name(name: &str, prefix: &str) -> Result<u64, SessionError> {
    let Some(rest) = name.strip_prefix(prefix) else {
        return Err(SessionError::PathEscape(PathBuf::from(name)));
    };
    if rest.is_empty() || rest.starts_with('0') || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SessionError::PathEscape(PathBuf::from(name)));
    }
    let key: u64 = rest
        .parse()
        .map_err(|_| SessionError::PathEscape(PathBuf::from(name)))?;
    if key == 0 {
        return Err(SessionError::PathEscape(PathBuf::from(name)));
    }
    Ok(key)
}

/// Returns an advisory host timestamp; it never enters canonical data.
pub(crate) fn now_unix_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

pub(crate) fn io_error(operation: &'static str, path: &Path, error: io::Error) -> SessionError {
    SessionError::Io {
        operation,
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};
    use std::path::Path;

    use super::{marker_bytes, parse_keyed_name, parse_marker, read_limited};
    use crate::error::SessionError;
    use crate::storage::STORAGE_LAYOUT_VERSION;

    struct FixedReader {
        remaining: usize,
    }

    impl Read for FixedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.remaining);
            buffer[..count].fill(b'x');
            self.remaining -= count;
            Ok(count)
        }
    }

    #[test]
    fn canonical_keyed_names_parse_and_noncanonical_names_are_rejected() {
        assert_eq!(parse_keyed_name("session-1", "session-"), Ok(1));
        assert_eq!(parse_keyed_name("snapshot-42", "snapshot-"), Ok(42));
        for invalid in [
            "session-",
            "session-0",
            "session-01",
            "session-+1",
            "session--1",
            "session-1x",
            "session- 1",
            "session-18446744073709551616",
            "other-1",
        ] {
            assert!(
                matches!(
                    parse_keyed_name(invalid, "session-"),
                    Err(SessionError::PathEscape(_))
                ),
                "{invalid} must be rejected"
            );
        }
    }

    #[test]
    fn bounded_reader_rejects_growth_after_the_length_check() {
        let reader = FixedReader { remaining: 100 };
        let error = read_limited(reader, 4, 8, Path::new("checkpoint.cfsnap"))
            .expect_err("growth past the limit must be rejected");
        assert!(matches!(error, SessionError::FileTooLarge { limit: 8, .. }));
    }

    #[test]
    fn marker_round_trips_and_rejects_other_layouts() {
        let bytes = marker_bytes();
        assert_eq!(parse_marker(&bytes), Ok(()));

        let mut wrong_magic = bytes;
        wrong_magic[0] ^= 0xff;
        assert!(matches!(
            parse_marker(&wrong_magic),
            Err(SessionError::NotAStorageRoot(_))
        ));

        let mut wrong_version = bytes;
        wrong_version[8..10].copy_from_slice(&(STORAGE_LAYOUT_VERSION + 1).to_le_bytes());
        assert_eq!(
            parse_marker(&wrong_version),
            Err(SessionError::UnsupportedLayoutVersion {
                found: STORAGE_LAYOUT_VERSION + 1,
                supported: STORAGE_LAYOUT_VERSION,
            })
        );
    }
}
