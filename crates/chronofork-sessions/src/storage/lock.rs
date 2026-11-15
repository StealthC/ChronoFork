//! Cross-process writer and key-allocation lock protocol.
//!
//! This module implements the ownership half of
//! [ADR-0003](../../../../docs/adr/0003-durable-session-storage.md) §10: a canonical
//! lock name is published by atomically hard-linking a fully written private record,
//! so readers of the canonical name never observe a half-written lock. Release and
//! recovery use an owner-checked rename-aside protocol and never delete a changed
//! successor.
//!
//! Ownership is cooperative metadata, not authentication and not a security
//! boundary. Acquisition never waits, never retries silently and never steals an
//! expired lease; an empty, partial or otherwise unreadable canonical lock is
//! reported as contention. Only the exact canonical names [`WRITER_LOCK_FILE`] and
//! [`ALLOCATION_LOCK_FILE`] are treated as locks; every other record in a session or
//! state directory is a private protocol artifact and is never a lock.
//!
//! Lock records are storage-level records built on [`chronofork_core::codec`]. They
//! never enter checkpoint bytes, state hashes, listing order or replay results. The
//! owner token and generation are derived from the process id and a process-local
//! monotonically increasing nonce; neither is derived from host time.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::hash_bytes;

use crate::error::SessionError;
use crate::storage::root;

/// Maximum size of a lock record, matching the ADR-0003 §11 limit table.
pub const MAX_LOCK_BYTES: u64 = 4 * 1024;

/// Canonical per-session writer lock file name.
pub(crate) const WRITER_LOCK_FILE: &str = "writer.lock";

/// Canonical root key-allocation lock file name.
pub(crate) const ALLOCATION_LOCK_FILE: &str = "allocation.lock";

/// Advisory lease applied to an ordinary writer-lock acquisition.
pub(crate) const DEFAULT_LEASE: Duration = Duration::from_secs(300);

const LOCK_MAGIC: [u8; 8] = *b"CFLOCK01";
const LOCK_FORMAT_VERSION: u16 = 1;
const LOCK_CHECKSUM_LEN: usize = 32;
const MAX_OWNER_TOKEN_BYTES: usize = 64;
const MAX_HOST_LABEL_BYTES: usize = 255;
const PRIVATE_RECORD_RETRIES: u8 = 4;

/// Smallest complete record: fixed fields with every optional field absent.
const MIN_LOCK_LEN: usize = LOCK_MAGIC.len()
    + 2 // lock format version
    + 2 // owner-token length
    + 1 // shortest owner token
    + 8 // generation
    + 4 // process id
    + 1 // host-label flag
    + 1 // acquisition-timestamp flag
    + 1 // lease-deadline flag
    + LOCK_CHECKSUM_LEN;

/// Source of process-local unique owner tokens.
///
/// This is a token generator, not a lock or a registry: it holds no session state,
/// is never used to serialize an operation, and never enters canonical data.
static NEXT_OWNER_NONCE: AtomicU64 = AtomicU64::new(1);

/// Owner token plus generation, the expected pair for release and recovery.
///
/// A token is chosen by storage at acquisition from a bounded safe alphabet and is
/// never a caller-supplied path component. The generation is a per-record `u64`;
/// together they identify one lock record. Neither value is canonical data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockIdentity {
    /// Bounded, path-safe owner token chosen at acquisition.
    pub owner_token: String,
    /// Per-record generation chosen at acquisition; always at least one.
    pub generation: u64,
}

impl fmt::Display for LockIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.owner_token, self.generation)
    }
}

/// A held per-session writer lock.
///
/// The lock is released explicitly through
/// [`SnapshotRepository::release_writer_lock`](crate::SnapshotRepository::release_writer_lock);
/// there is no hidden release on drop, so a leaked handle is an explicit, diagnosable
/// stale lock rather than silent state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLock {
    session_key: u64,
    store_root: PathBuf,
    identity: LockIdentity,
    lease_expires_at_unix_ms: Option<u64>,
}

impl WriterLock {
    pub(crate) fn new(session_key: u64, store_root: &Path, record: &LockRecord) -> Self {
        Self {
            session_key,
            store_root: store_root.to_path_buf(),
            identity: record.identity.clone(),
            lease_expires_at_unix_ms: record.lease_expires_at_unix_ms,
        }
    }

    /// Returns the owner token and generation recorded for this lock.
    #[must_use]
    pub fn identity(&self) -> &LockIdentity {
        &self.identity
    }

    /// Returns the advisory lease deadline in host milliseconds, if a host clock was
    /// available at acquisition. The deadline is advisory metadata only.
    #[must_use]
    pub fn lease_expires_at_unix_ms(&self) -> Option<u64> {
        self.lease_expires_at_unix_ms
    }

    pub(crate) fn session_key(&self) -> u64 {
        self.session_key
    }

    pub(crate) fn store_root(&self) -> &Path {
        &self.store_root
    }
}

/// One complete, validated lock record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LockRecord {
    pub(crate) identity: LockIdentity,
    pub(crate) process_id: u32,
    pub(crate) host_label: Option<String>,
    pub(crate) acquired_at_unix_ms: Option<u64>,
    pub(crate) lease_expires_at_unix_ms: Option<u64>,
}

impl LockRecord {
    /// Returns whether an advisory lease deadline has passed.
    ///
    /// An absent deadline or an unavailable host clock is treated as unexpired, so
    /// recovery still requires explicit operator force. An expired lease is a
    /// diagnostic only and never authorizes acquisition.
    pub(crate) fn lease_expired(&self) -> bool {
        match self.lease_expires_at_unix_ms {
            Some(deadline) => root::now_unix_ms().is_some_and(|now| now >= deadline),
            None => false,
        }
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SessionError> {
        self.validate()
            .map_err(|message| SessionError::InvalidName(format!("lock record: {message}")))?;
        let mut writer = Writer::with_capacity(MIN_LOCK_LEN);
        writer.write_bytes(&LOCK_MAGIC);
        writer.write_u16_le(LOCK_FORMAT_VERSION);
        writer
            .write_string_u16(&self.identity.owner_token)
            .map_err(codec_error)?;
        writer.write_u64_le(self.identity.generation);
        writer.write_u32_le(self.process_id);
        write_optional_string(&mut writer, self.host_label.as_deref())?;
        write_optional_u64(&mut writer, self.acquired_at_unix_ms);
        write_optional_u64(&mut writer, self.lease_expires_at_unix_ms);
        let checksum = hash_bytes(writer.as_slice());
        writer.write_bytes(checksum.as_bytes());
        Ok(writer.into_bytes())
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < MIN_LOCK_LEN {
            return Err(format!(
                "lock record is {} bytes, minimum is {MIN_LOCK_LEN}",
                bytes.len()
            ));
        }
        let body_len = bytes.len() - LOCK_CHECKSUM_LEN;
        let (body, checksum) = bytes.split_at(body_len);
        if hash_bytes(body).as_bytes() != checksum {
            return Err("lock record checksum mismatch".to_owned());
        }
        let mut reader = Reader::new(body);
        let magic = reader
            .read_array::<8>()
            .map_err(|error| format!("lock record is malformed: {error}"))?;
        if magic != LOCK_MAGIC {
            return Err("lock record magic mismatch".to_owned());
        }
        let version = reader
            .read_u16_le()
            .map_err(|error| format!("lock record is malformed: {error}"))?;
        if version != LOCK_FORMAT_VERSION {
            return Err(format!(
                "unsupported lock record version {version}, expected {LOCK_FORMAT_VERSION}"
            ));
        }
        let owner_token = read_string(&mut reader)?;
        let generation = reader
            .read_u64_le()
            .map_err(|error| format!("lock record is malformed: {error}"))?;
        let process_id = reader
            .read_u32_le()
            .map_err(|error| format!("lock record is malformed: {error}"))?;
        let host_label = read_optional_string(&mut reader)?;
        let acquired_at_unix_ms = read_optional_u64(&mut reader)?;
        let lease_expires_at_unix_ms = read_optional_u64(&mut reader)?;
        reader
            .finish()
            .map_err(|error| format!("lock record has trailing bytes: {error}"))?;
        let record = Self {
            identity: LockIdentity {
                owner_token,
                generation,
            },
            process_id,
            host_label,
            acquired_at_unix_ms,
            lease_expires_at_unix_ms,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), String> {
        validate_token(&self.identity.owner_token)?;
        if self.identity.generation == 0 {
            return Err("owner generation must be at least one".to_owned());
        }
        if let Some(label) = self.host_label.as_deref() {
            if label.len() > MAX_HOST_LABEL_BYTES {
                return Err(format!(
                    "host label is {} bytes, limit is {MAX_HOST_LABEL_BYTES}",
                    label.len()
                ));
            }
            if label.chars().any(char::is_control) {
                return Err("host label contains a control character".to_owned());
            }
        }
        Ok(())
    }
}

/// Observed state of a canonical or private lock name.
enum LockState {
    Missing,
    Complete(LockRecord),
    Corrupt(String),
    Symlink(PathBuf),
    WrongType(PathBuf),
    Io(SessionError),
}

/// Acquires a lock by atomic hard-link publication of a complete private record.
///
/// On success the canonical lock name is visible only with complete content. On
/// contention the observed owner is returned, except when the existing record is
/// unreadable: occupancy is authoritative, so it is still contention with no owner.
pub(crate) fn acquire(
    directory: &Path,
    lock_name: &str,
    session: Option<u64>,
    lease: Duration,
) -> Result<LockRecord, SessionError> {
    let canonical = directory.join(lock_name);
    let mut retried_after_disappearance = false;
    loop {
        let (record, temporary) = publish_candidate(directory, lock_name, lease)?;

        match fs::hard_link(&temporary, &canonical) {
            Ok(()) => {
                let _ = fs::remove_file(&temporary);
                return Ok(record);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let observed = inspect_lock(&canonical);
                let _ = fs::remove_file(&temporary);
                match observed {
                    LockState::Missing => {
                        if retried_after_disappearance {
                            return Err(contended(session, None));
                        }
                        retried_after_disappearance = true;
                        continue;
                    }
                    LockState::Complete(record) => {
                        return Err(contended(session, Some(record.identity)));
                    }
                    LockState::Symlink(path) => return Err(SessionError::SymlinkRejected(path)),
                    LockState::WrongType(path) => return Err(SessionError::WrongFileType(path)),
                    LockState::Corrupt(_) | LockState::Io(_) => {
                        return Err(contended(session, None));
                    }
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(publication_error(&canonical, error));
            }
        }
    }
}

/// Releases a lock whose expected owner/generation pair still matches.
pub(crate) fn release(
    directory: &Path,
    lock_name: &str,
    session: Option<u64>,
    expected: &LockIdentity,
) -> Result<(), SessionError> {
    remove_owned(
        directory,
        lock_name,
        session,
        expected,
        OwnershipMode::Release,
    )
}

/// Explicitly recovers a lock, requiring the observed owner/generation pair.
///
/// A non-expired lease returns [`SessionError::WriterContended`] unless `force`
/// confirms that no writer is live. Forcing still requires the owner/generation
/// check and the same rename-aside protocol, so a changed successor is preserved.
pub(crate) fn recover(
    directory: &Path,
    lock_name: &str,
    session: Option<u64>,
    expected: &LockIdentity,
    force: bool,
) -> Result<(), SessionError> {
    remove_owned(
        directory,
        lock_name,
        session,
        expected,
        OwnershipMode::Recovery { force },
    )
}

/// Verifies that an already-held handle still owns the canonical lock record.
///
/// This is not acquisition and never waits: it is a fail-closed check for a
/// lock-aware mutating operation that receives a caller-held [`WriterLock`]. A
/// missing, changed or unreadable canonical record returns
/// [`SessionError::NotLockOwner`] and nothing is removed.
pub(crate) fn verify_held(
    directory: &Path,
    lock_name: &str,
    session: Option<u64>,
    expected: &LockIdentity,
) -> Result<(), SessionError> {
    match inspect_lock(&directory.join(lock_name)) {
        LockState::Complete(record) if record.identity == *expected => Ok(()),
        LockState::Missing
        | LockState::Complete(_)
        | LockState::Corrupt(_)
        | LockState::Symlink(_)
        | LockState::WrongType(_)
        | LockState::Io(_) => Err(not_owner(session, expected)),
    }
}

/// Returns whether a directory entry name is a private lock-protocol artifact.
///
/// Only the exact canonical names are locks; every `writer.lock.*` or
/// `allocation.lock.*` record is a leftover that must never be treated as a lock.
pub(crate) fn is_private_lock_record(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(WRITER_LOCK_FILE)
        .or_else(|| name.strip_prefix(ALLOCATION_LOCK_FILE))
    else {
        return false;
    };
    rest.starts_with('.')
}

#[derive(Clone, Copy)]
enum OwnershipMode {
    Release,
    Recovery { force: bool },
}

fn remove_owned(
    directory: &Path,
    lock_name: &str,
    session: Option<u64>,
    expected: &LockIdentity,
    mode: OwnershipMode,
) -> Result<(), SessionError> {
    let canonical = directory.join(lock_name);
    let observed = match inspect_lock(&canonical) {
        LockState::Missing => return Err(not_owner(session, expected)),
        LockState::Complete(record) => record,
        LockState::Symlink(path) => {
            return Err(unreadable_owned(
                mode,
                session,
                expected,
                &path,
                "lock record is a symbolic link",
            ));
        }
        LockState::WrongType(path) => {
            return Err(unreadable_owned(
                mode,
                session,
                expected,
                &path,
                "lock record is not a regular file",
            ));
        }
        LockState::Corrupt(message) => {
            return Err(unreadable_owned(
                mode,
                session,
                expected,
                &canonical,
                message.as_str(),
            ));
        }
        LockState::Io(error) => {
            return Err(unreadable_owned(
                mode,
                session,
                expected,
                &canonical,
                error.to_string().as_str(),
            ));
        }
    };
    if observed.identity != *expected {
        return Err(not_owner(session, expected));
    }
    if let OwnershipMode::Recovery { force } = mode {
        if !force && !observed.lease_expired() {
            return Err(contended(session, Some(observed.identity)));
        }
    }

    let verb = match mode {
        OwnershipMode::Release => "releasing",
        OwnershipMode::Recovery { .. } => "recovering",
    };
    let (token, _) = new_identity();
    let private = directory.join(format!("{lock_name}.{verb}.{token}"));
    match fs::rename(&canonical, &private) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(not_owner(session, expected));
        }
        Err(error) => {
            return Err(root::io_error(
                "rename lock record aside",
                &canonical,
                error,
            ));
        }
    }

    match inspect_lock(&private) {
        LockState::Complete(record) if record.identity == *expected => {
            let _ = fs::remove_file(&private);
            Ok(())
        }
        LockState::Complete(_) => {
            restore_private_record(&private, &canonical)?;
            Err(not_owner(session, expected))
        }
        LockState::Missing => Err(unreadable_owned(
            mode,
            session,
            expected,
            &canonical,
            "lock record disappeared during release or recovery",
        )),
        LockState::Symlink(path) => {
            restore_private_record(&private, &canonical)?;
            Err(unreadable_owned(
                mode,
                session,
                expected,
                &path,
                "renamed lock record is a symbolic link",
            ))
        }
        LockState::WrongType(path) => {
            restore_private_record(&private, &canonical)?;
            Err(unreadable_owned(
                mode,
                session,
                expected,
                &path,
                "renamed lock record is not a regular file",
            ))
        }
        LockState::Corrupt(message) => {
            restore_private_record(&private, &canonical)?;
            Err(unreadable_owned(
                mode,
                session,
                expected,
                &canonical,
                message.as_str(),
            ))
        }
        LockState::Io(error) => {
            restore_private_record(&private, &canonical)?;
            Err(unreadable_owned(
                mode,
                session,
                expected,
                &canonical,
                error.to_string().as_str(),
            ))
        }
    }
}

fn new_record(lease: Duration) -> Result<(LockRecord, Vec<u8>), SessionError> {
    let (owner_token, generation) = new_identity();
    let now = root::now_unix_ms();
    let record = LockRecord {
        identity: LockIdentity {
            owner_token,
            generation,
        },
        process_id: std::process::id(),
        host_label: host_label(),
        acquired_at_unix_ms: now,
        lease_expires_at_unix_ms: now.map(|now| {
            let millis = u64::try_from(lease.as_millis()).unwrap_or(u64::MAX);
            now.saturating_add(millis)
        }),
    };
    let bytes = record.encode()?;
    Ok((record, bytes))
}

/// Writes a fresh private candidate record, retrying a name collision with a new
/// token instead of overwriting anything. After the bounded retries this is `Io`.
fn publish_candidate(
    directory: &Path,
    lock_name: &str,
    lease: Duration,
) -> Result<(LockRecord, PathBuf), SessionError> {
    for _ in 0..=PRIVATE_RECORD_RETRIES {
        let (record, bytes) = new_record(lease)?;
        let temporary = directory.join(format!("{lock_name}.{}.tmp", record.identity.owner_token));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(|error| {
                    root::io_error("write private lock record", &temporary, error)
                })?;
                file.sync_all().map_err(|error| {
                    root::io_error("sync private lock record", &temporary, error)
                })?;
                return Ok((record, temporary));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(root::io_error(
                    "create private lock record",
                    &temporary,
                    error,
                ));
            }
        }
    }
    Err(SessionError::Io {
        operation: "create private lock record",
        path: directory.to_path_buf(),
        kind: ErrorKind::AlreadyExists,
    })
}

/// Generates a unique owner token and generation without host time.
fn new_identity() -> (String, u64) {
    let nonce = NEXT_OWNER_NONCE.fetch_add(1, Ordering::Relaxed);
    let process = u64::from(std::process::id());
    (format!("{process:08x}{nonce:016x}"), nonce)
}

fn host_label() -> Option<String> {
    for key in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(key) {
            let label: String = value
                .chars()
                .filter(|character| !character.is_control())
                .take(MAX_HOST_LABEL_BYTES)
                .collect();
            if !label.is_empty() {
                return Some(label);
            }
        }
    }
    None
}

fn inspect_lock(path: &Path) -> LockState {
    match root::entry_metadata(path) {
        Err(error) => LockState::Io(error),
        Ok(None) => LockState::Missing,
        Ok(Some(metadata)) if metadata.file_type().is_symlink() => {
            LockState::Symlink(path.to_path_buf())
        }
        Ok(Some(metadata)) if !metadata.is_file() => LockState::WrongType(path.to_path_buf()),
        Ok(Some(_)) => match root::read_bounded_file(path, MAX_LOCK_BYTES, "read lock record") {
            Ok(bytes) => match LockRecord::decode(&bytes) {
                Ok(record) => LockState::Complete(record),
                Err(message) => LockState::Corrupt(message),
            },
            Err(error) => LockState::Io(error),
        },
    }
}

/// Restores a renamed-aside record to its canonical name without deleting anything
/// when a successor already exists.
fn restore_private_record(private: &Path, canonical: &Path) -> Result<(), SessionError> {
    match fs::hard_link(private, canonical) {
        Ok(()) => {
            let _ = fs::remove_file(private);
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(root::io_error("restore lock record", canonical, error)),
    }
}

fn contended(session: Option<u64>, owner: Option<LockIdentity>) -> SessionError {
    SessionError::WriterContended { session, owner }
}

fn not_owner(session: Option<u64>, expected: &LockIdentity) -> SessionError {
    SessionError::NotLockOwner {
        session,
        expected: expected.clone(),
    }
}

fn unreadable_owned(
    mode: OwnershipMode,
    session: Option<u64>,
    expected: &LockIdentity,
    path: &Path,
    message: &str,
) -> SessionError {
    match mode {
        OwnershipMode::Release => not_owner(session, expected),
        OwnershipMode::Recovery { .. } => SessionError::CorruptLock {
            path: path.to_path_buf(),
            message: message.to_owned(),
        },
    }
}

/// Classifies a failed hard-link publication.
///
/// Filesystems without hard-link support report `Unsupported`, `PermissionDenied` or
/// a cross-device error depending on the platform; all three fail closed with
/// [`SessionError::LockPrimitiveUnsupported`] instead of falling back to a
/// non-atomic `create_new`-then-write publication.
fn publication_error(canonical: &Path, error: io::Error) -> SessionError {
    if link_primitive_unsupported(error.kind()) {
        SessionError::LockPrimitiveUnsupported {
            path: canonical.to_path_buf(),
        }
    } else {
        root::io_error("publish lock record", canonical, error)
    }
}

fn link_primitive_unsupported(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Unsupported | ErrorKind::CrossesDevices | ErrorKind::PermissionDenied
    )
}

fn codec_error(error: CodecError) -> SessionError {
    SessionError::InvalidName(format!("lock record is not serializable: {error}"))
}

fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("owner token must not be empty".to_owned());
    }
    if token.len() > MAX_OWNER_TOKEN_BYTES {
        return Err(format!(
            "owner token is {} bytes, limit is {MAX_OWNER_TOKEN_BYTES}",
            token.len()
        ));
    }
    if !token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("owner token must use the bounded safe alphabet".to_owned());
    }
    Ok(())
}

fn write_optional_string(writer: &mut Writer, value: Option<&str>) -> Result<(), SessionError> {
    match value {
        Some(value) => {
            writer.write_u8(1);
            writer.write_string_u16(value).map_err(codec_error)
        }
        None => {
            writer.write_u8(0);
            Ok(())
        }
    }
}

fn write_optional_u64(writer: &mut Writer, value: Option<u64>) {
    match value {
        Some(value) => {
            writer.write_u8(1);
            writer.write_u64_le(value);
        }
        None => writer.write_u8(0),
    }
}

fn read_string(reader: &mut Reader<'_>) -> Result<String, String> {
    reader
        .read_string_u16()
        .map(str::to_owned)
        .map_err(|error| format!("lock record is malformed: {error}"))
}

fn read_optional_string(reader: &mut Reader<'_>) -> Result<Option<String>, String> {
    match read_flag(reader)? {
        false => Ok(None),
        true => Ok(Some(read_string(reader)?)),
    }
}

fn read_optional_u64(reader: &mut Reader<'_>) -> Result<Option<u64>, String> {
    match read_flag(reader)? {
        false => Ok(None),
        true => reader
            .read_u64_le()
            .map(Some)
            .map_err(|error| format!("lock record is malformed: {error}")),
    }
}

fn read_flag(reader: &mut Reader<'_>) -> Result<bool, String> {
    match reader
        .read_u8()
        .map_err(|error| format!("lock record is malformed: {error}"))?
    {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("invalid lock record flag {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::path::Path;
    use std::time::Duration;

    use super::{
        LockIdentity, LockRecord, is_private_lock_record, link_primitive_unsupported, new_identity,
        new_record, publication_error, validate_token,
    };
    use crate::error::SessionError;

    fn record() -> LockRecord {
        LockRecord {
            identity: LockIdentity {
                owner_token: "deadbeefcafebabe".to_owned(),
                generation: 7,
            },
            process_id: 42,
            host_label: Some("test-host".to_owned()),
            acquired_at_unix_ms: Some(1_700_000_000_000),
            lease_expires_at_unix_ms: Some(1_700_000_300_000),
        }
    }

    #[test]
    fn lock_records_round_trip_and_reject_corruption() {
        let record = record();
        let bytes = record.encode().expect("encode lock record");
        assert_eq!(LockRecord::decode(&bytes), Ok(record));

        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xff;
        assert!(LockRecord::decode(&flipped).is_err());

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(LockRecord::decode(&trailing).is_err());

        assert!(LockRecord::decode(b"").is_err());
        assert!(LockRecord::decode(b"CFLOCK01").is_err());
    }

    #[test]
    fn owner_tokens_are_unique_and_path_safe() {
        let (first_token, first_generation) = new_identity();
        let (second_token, second_generation) = new_identity();
        assert_ne!(first_token, second_token);
        assert_ne!(first_generation, second_generation);
        for token in [first_token, second_token] {
            assert_eq!(validate_token(&token), Ok(()));
            assert!(
                token
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b'-'),
                "a token must be filesystem-safe"
            );
        }
    }

    #[test]
    fn unsafe_owner_tokens_are_rejected() {
        for token in [
            "",
            "owner/token",
            "owner token",
            "owner.token",
            "owner\0token",
        ] {
            assert!(validate_token(token).is_err(), "{token:?} must be rejected");
        }
        assert!(validate_token(&"a".repeat(65)).is_err());
    }

    #[test]
    fn lock_record_fields_are_bounded() {
        let mut overlong = record();
        overlong.identity.owner_token = "a".repeat(65);
        assert!(overlong.encode().is_err());

        let mut zero_generation = record();
        zero_generation.identity.generation = 0;
        assert!(zero_generation.encode().is_err());

        let mut control_label = record();
        control_label.host_label = Some("bad\nlabel".to_owned());
        assert!(control_label.encode().is_err());
    }

    #[test]
    fn hard_link_failure_classification_fails_closed() {
        let path = Path::new("writer.lock");
        for kind in [
            ErrorKind::Unsupported,
            ErrorKind::PermissionDenied,
            ErrorKind::CrossesDevices,
        ] {
            assert!(link_primitive_unsupported(kind), "{kind:?} is unsupported");
            assert!(matches!(
                publication_error(path, std::io::Error::from(kind)),
                SessionError::LockPrimitiveUnsupported { .. }
            ));
        }
        assert!(!link_primitive_unsupported(ErrorKind::NotFound));
        assert!(matches!(
            publication_error(path, std::io::Error::from(ErrorKind::NotFound)),
            SessionError::Io { .. }
        ));
    }

    #[test]
    fn only_dotted_private_protocol_names_are_leftovers() {
        for name in [
            "writer.lock.deadbeef.tmp",
            "writer.lock.releasing.deadbeef",
            "writer.lock.recovering.deadbeef",
            "allocation.lock.deadbeef.tmp",
            "allocation.lock.releasing.deadbeef",
        ] {
            assert!(is_private_lock_record(name), "{name} must be private");
        }
        for name in [
            "writer.lock",
            "allocation.lock",
            "writer.locks",
            "writerlock.foo",
            "session.cfmeta",
        ] {
            assert!(!is_private_lock_record(name), "{name} is not private");
        }
    }

    #[test]
    fn generated_record_has_an_advisory_lease_deadline() {
        let (record, bytes) = new_record(Duration::ZERO).expect("new record");
        assert!(!bytes.is_empty());
        let decoded = LockRecord::decode(&bytes).expect("decode generated record");
        assert_eq!(decoded, record);
        assert!(decoded.lease_expired(), "a zero lease expires immediately");
    }
}
