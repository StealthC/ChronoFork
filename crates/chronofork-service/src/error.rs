//! Typed service errors.
//!
//! A [`ServiceError`] carries a stable machine-readable code, a bounded message and
//! an explicit retryable flag. Transport adapters map [`ErrorCode`] to an HTTP status
//! but never invent a new code or reinterpret a message.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_sessions::{SessionError, StorageAnomalyKind};

use crate::json::JsonValue;
use crate::limits::ServiceLimits;

/// Schema name of the canonical error envelope.
///
/// The envelope schema string is unchanged: the v2 additions are **additive codes
/// in the same envelope** (`cancelled`, `deadline`, `unknown_outcome`), and the
/// wire version that declares them is carried as `wire_version` beside the schema
/// and advertised by `capabilities` as `api_version`. A consumer that treats an
/// unknown code as an opaque string keeps working; one that must branch on the code
/// reads the declared version instead of guessing.
pub const ERROR_SCHEMA: &str = "chronofork.error.v1";

/// Wire version of the API this build speaks (ADR-0016 §Compatibility).
///
/// Version 2 adds the control operations (`session.cancel`, `session.progress`,
/// `session.operation`), the reconciliation fields, the `performed`/`stop_reason`
/// run result and the `cancelled`/`deadline`/`unknown_outcome` codes; version 1 is
/// the pre-ADR-0016 surface.
pub const WIRE_VERSION: u64 = 2;

/// Stable machine-readable error codes shared by every transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    /// The request is malformed or fails a documented input rule.
    InvalidRequest,
    /// The operation name is not in the catalog.
    UnknownOp,
    /// The bearer token is missing or unknown.
    Unauthorized,
    /// The principal lacks a scope or does not own the session.
    Forbidden,
    /// The referenced session, snapshot, durable key or artifact does not exist.
    NotFound,
    /// The request conflicts with current state (contended writer, halted or closed).
    Conflict,
    /// The operation needs a capability this service instance was not configured with.
    NotConfigured,
    /// A documented resource bound was exceeded.
    LimitExceeded,
    /// The session is closed.
    Closed,
    /// The machine has halted and cannot advance.
    Halted,
    /// The operation was cancelled by an explicit `session.cancel` request.
    ///
    /// Wire addition of API version 2 (ADR-0016 §2). A `session.run` truncated by a
    /// cancellation returns its partial result with `stop_reason: "cancelled"`; this
    /// code is what a request that could not start, or a control read that names a
    /// cancelled operation, reports.
    Cancelled,
    /// The operation exceeded its host wall-clock deadline.
    ///
    /// Wire addition of API version 2 (ADR-0016 §2). A `session.run` truncated by its
    /// deadline returns `performed`/`tick` with `stop_reason: "deadline"`; the code is
    /// used where a deadline prevents an operation from being admitted at all.
    Deadline,
    /// The result of a request key is no longer retained in this process epoch.
    ///
    /// Wire addition of API version 2 (ADR-0016 §2, §6): the reconciliation horizon
    /// is bounded by `operation_ttl` (counted from completion) and by the process
    /// epoch. Beyond it the server reports the outcome as unknown and **does not
    /// re-execute** the operation.
    UnknownOutcome,
    /// A backend rejected the operation.
    Backend,
    /// The durable storage layer rejected the operation.
    Storage,
    /// The request declares an API wire version this server does not implement.
    ///
    /// Wire addition of API version 2 (ADR-0016 §Compatibility/F10): the refusal happens
    /// **before dispatch**, so an incompatible call never applies a partial mutation.
    UnsupportedVersion,
    /// A defect in the service itself; never a normal client outcome.
    Internal,
}

impl ErrorCode {
    /// Returns the stable wire string for this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnknownOp => "unknown_op",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::NotConfigured => "not_configured",
            Self::LimitExceeded => "limit_exceeded",
            Self::Closed => "closed",
            Self::Halted => "halted",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
            Self::UnknownOutcome => "unknown_outcome",
            Self::Backend => "backend_error",
            Self::Storage => "storage_error",
            Self::UnsupportedVersion => "unsupported_version",
            Self::Internal => "internal_error",
        }
    }

    /// Parses the stable wire string back into a code.
    ///
    /// This is the inverse of [`ErrorCode::as_str`] and exists so a stored canonical
    /// error envelope can be returned as the same typed error it was built from.
    /// An unknown string is `None`: a consumer never invents a code.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "invalid_request" => Self::InvalidRequest,
            "unknown_op" => Self::UnknownOp,
            "unauthorized" => Self::Unauthorized,
            "forbidden" => Self::Forbidden,
            "not_found" => Self::NotFound,
            "conflict" => Self::Conflict,
            "not_configured" => Self::NotConfigured,
            "limit_exceeded" => Self::LimitExceeded,
            "closed" => Self::Closed,
            "halted" => Self::Halted,
            "cancelled" => Self::Cancelled,
            "deadline" => Self::Deadline,
            "unknown_outcome" => Self::UnknownOutcome,
            "backend_error" => Self::Backend,
            "storage_error" => Self::Storage,
            "unsupported_version" => Self::UnsupportedVersion,
            "internal_error" => Self::Internal,
            _ => return None,
        })
    }
}

/// A typed service failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceError {
    code: ErrorCode,
    message: String,
    retryable: bool,
}

impl ServiceError {
    /// Creates a non-retryable error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    /// Creates an invalid-request error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    /// Creates an internal-defect error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// Creates a conflict that a client may retry without changing the request.
    #[must_use]
    pub fn retryable_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message).with_retryable(true)
    }

    /// Returns the stable error code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Returns the bounded human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether the same request may succeed later without changes.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Marks this error retryable.
    #[must_use]
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Builds the canonical `chronofork.error.v1` JSON value.
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        self.to_json_versioned(crate::version::ApiVersion::V2)
    }

    /// Builds the error envelope for one API wire version.
    ///
    /// The v2 envelope declares `wire_version: 2`; the v1 envelope is the original
    /// `chronofork.error.v1` document without it (ADR-0016 §Compatibility).
    pub fn to_json_versioned(&self, version: crate::version::ApiVersion) -> JsonValue {
        let mut error = BTreeMap::new();
        error.insert(
            "code".to_owned(),
            JsonValue::String(self.code.as_str().to_owned()),
        );
        error.insert(
            "message".to_owned(),
            JsonValue::String(self.message.clone()),
        );
        error.insert("retryable".to_owned(), JsonValue::Bool(self.retryable));
        let mut root = BTreeMap::new();
        root.insert(
            "schema".to_owned(),
            JsonValue::String(ERROR_SCHEMA.to_owned()),
        );
        root.insert(
            "wire_version".to_owned(),
            JsonValue::Int(WIRE_VERSION as i64),
        );
        root.insert("error".to_owned(), JsonValue::Object(error));
        if !version.is_v2() {
            root.remove("wire_version");
        }
        JsonValue::Object(root)
    }

    /// Builds the canonical error envelope bytes.
    pub fn to_canonical_bytes(&self, limits: &ServiceLimits) -> Result<Vec<u8>, ServiceError> {
        crate::json::to_canonical_bytes(&self.to_json(), limits)
    }

    /// Builds the canonical error envelope bytes for one API wire version.
    pub fn to_canonical_bytes_versioned(
        &self,
        limits: &ServiceLimits,
        version: crate::version::ApiVersion,
    ) -> Result<Vec<u8>, ServiceError> {
        crate::json::to_canonical_bytes(&self.to_json_versioned(version), limits)
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ServiceError {}

impl From<SessionError> for ServiceError {
    /// Maps a session or storage failure onto the stable service taxonomy.
    ///
    /// The durable CLI's established human diagnostics survive this boundary: a
    /// variant whose `Display` text is already bounded and path-free keeps that exact
    /// text, while every variant that carries a host path, a lock-owner identity or an
    /// oversized payload is rendered as a fixed phrase. No message produced here can
    /// contain a host path, a temporary-directory fragment, a lock owner token, raw
    /// backend payload bytes or token material.
    ///
    /// The match is exhaustive with no wildcard arm, so a new session-layer variant is
    /// a compile error instead of a silently reused (and possibly path-bearing)
    /// `Display` string.
    fn from(error: SessionError) -> Self {
        let (code, message, retryable) = match &error {
            // Path-free `Display` text, reused verbatim for the durable diagnostics.
            SessionError::UnknownSession(_) => reused(ErrorCode::NotFound, &error),
            SessionError::UnknownSnapshot { .. } => reused(ErrorCode::NotFound, &error),
            SessionError::Closed(_) => reused(ErrorCode::Closed, &error),
            SessionError::NotRunning { .. } => reused(ErrorCode::Conflict, &error),
            SessionError::Halted(_) => reused(ErrorCode::Halted, &error),
            // A per-session quota rejection names the quota, its limit and the value
            // the operation would have reached; none of them is client-controlled.
            SessionError::LimitExceeded { .. } => reused(ErrorCode::LimitExceeded, &error),
            // A missing backend declaration is reported under the byte quota: the capture
            // could not be preflighted, so nothing was allocated.
            SessionError::UnboundedSnapshot { .. } => reused(ErrorCode::LimitExceeded, &error),
            // A poisoned per-session owner is a service defect, never a normal client
            // outcome; the text carries only the session identifier.
            SessionError::Poisoned(_) => reused(ErrorCode::Internal, &error),
            SessionError::IdSpaceExhausted => reused(ErrorCode::LimitExceeded, &error),
            SessionError::InvalidSnapshot(_) => reused(ErrorCode::Storage, &error),
            SessionError::IncompatibleSnapshot(_) => reused(ErrorCode::Storage, &error),
            SessionError::Backend(_) => reused(ErrorCode::Backend, &error),
            SessionError::Trace(_) => reused(ErrorCode::Backend, &error),
            SessionError::NotInitialized => reused(ErrorCode::Storage, &error),
            SessionError::UnsupportedLayoutVersion { .. } => reused(ErrorCode::Storage, &error),
            SessionError::CorruptMetadata(_) => reused(ErrorCode::Storage, &error),
            SessionError::UnsupportedMetadataVersion { .. } => reused(ErrorCode::Storage, &error),
            SessionError::MetadataMismatch(_) => reused(ErrorCode::Storage, &error),
            SessionError::IdentityMismatch(_) => reused(ErrorCode::Storage, &error),
            SessionError::UnknownStorageSession(_) => reused(ErrorCode::NotFound, &error),
            SessionError::UnknownStorageSnapshot { .. } => reused(ErrorCode::NotFound, &error),
            SessionError::SessionConflict(_) => reused(ErrorCode::Conflict, &error),
            SessionError::SnapshotExists { .. } => reused(ErrorCode::Conflict, &error),
            SessionError::SessionNotEmpty(_) => reused(ErrorCode::Conflict, &error),
            SessionError::ListLimitExceeded { .. } => reused(ErrorCode::LimitExceeded, &error),
            SessionError::InvalidName(_) => reused(ErrorCode::InvalidRequest, &error),
            SessionError::CorruptAllocationCounter(_) => reused(ErrorCode::Storage, &error),

            // Variants whose `Display` carries a host path or an unbounded detail are
            // rendered as fixed, bounded phrases instead.
            SessionError::NotAStorageRoot(_) => {
                fixed(ErrorCode::Storage, "not a ChronoFork storage root")
            }
            SessionError::RootNotEmpty(_) => fixed(ErrorCode::Storage, "storage root is not empty"),
            SessionError::MissingRecord(_) => fixed(ErrorCode::Storage, "missing storage record"),
            SessionError::FileTooLarge { limit, found, .. } => (
                ErrorCode::LimitExceeded,
                format!("file is {found} bytes, limit is {limit}"),
                false,
            ),
            SessionError::PathEscape(_) => {
                fixed(ErrorCode::Storage, "managed path escapes the storage root")
            }
            SessionError::SymlinkRejected(_) => {
                fixed(ErrorCode::Storage, "managed path is a symbolic link")
            }
            SessionError::WrongFileType(_) => {
                fixed(ErrorCode::Storage, "managed path has the wrong file type")
            }
            SessionError::DurabilityUnconfirmed { operation, .. } => (
                ErrorCode::Storage,
                format!("{operation} is visible but not durably confirmed"),
                false,
            ),
            SessionError::WriterContended { session, .. } => (
                ErrorCode::Conflict,
                match session {
                    Some(key) => format!("durable session {key} writer lock is held by owner"),
                    None => "storage key-allocation lock is held by owner".to_owned(),
                },
                true,
            ),
            SessionError::NotLockOwner { session, .. } => (
                ErrorCode::Conflict,
                match session {
                    Some(key) => format!("durable session {key} writer lock ownership was lost"),
                    None => "storage key-allocation lock ownership was lost".to_owned(),
                },
                true,
            ),
            SessionError::CorruptLock { message, .. } => (
                ErrorCode::Storage,
                format!("unreadable lock record: {message}"),
                false,
            ),
            SessionError::LockPrimitiveUnsupported { .. } => fixed(
                ErrorCode::Storage,
                "filesystem does not support hard links, so lock publication cannot be atomic",
            ),
            SessionError::Io { .. } => fixed(ErrorCode::Storage, "storage operation failed"),
        };
        ServiceError::new(code, message).with_retryable(retryable)
    }
}

/// Keeps the session layer's path-free `Display` text for one mapped code.
fn reused(code: ErrorCode, error: &SessionError) -> (ErrorCode, String, bool) {
    (code, error.to_string(), false)
}

/// Uses one fixed, bounded phrase for a path-bearing or unbounded variant.
fn fixed(code: ErrorCode, message: &str) -> (ErrorCode, String, bool) {
    (code, message.to_owned(), false)
}

/// Maximum number of distinct anomaly labels one `store.list` response carries.
pub(crate) const MAX_ANOMALY_LABELS: usize = 16;

/// Returns a short, stable, path-free label for one storage anomaly kind.
///
/// The label names the category only. The path of the anomalous entry, the lock owner
/// identity and any embedded detail are never part of it.
pub(crate) fn anomaly_label(kind: &StorageAnomalyKind) -> &'static str {
    match kind {
        StorageAnomalyKind::IncompletePublish => "incomplete publish",
        StorageAnomalyKind::PendingDeletion => "pending deletion",
        StorageAnomalyKind::UnexpectedEntry => "unexpected entry",
        StorageAnomalyKind::LockProtocolRecord => "lock protocol record",
        StorageAnomalyKind::FutureLayoutVersion(_) => "future layout version",
        StorageAnomalyKind::InvalidKey => "invalid key",
        StorageAnomalyKind::CorruptObject(error) => storage_label(error),
    }
}

/// Returns the anomaly-label companion of [`From<SessionError>`] for one failure.
///
/// This is deliberately exhaustive and separate from the diagnostic mapping: it names
/// the category without the reason, path or owner identity a client must never see.
fn storage_label(error: &SessionError) -> &'static str {
    match error {
        SessionError::UnknownSession(_) => "unknown session",
        SessionError::UnknownSnapshot { .. } => "unknown snapshot",
        SessionError::Closed(_) => "session is closed",
        SessionError::NotRunning { .. } => "session is not running",
        SessionError::Halted(_) => "machine is halted",
        SessionError::LimitExceeded { .. } => "session quota exceeded",
        SessionError::UnboundedSnapshot { .. } => "snapshot payload bound not declared",
        SessionError::Poisoned(_) => "session owner poisoned",
        SessionError::IdSpaceExhausted => "identifier space exhausted",
        SessionError::InvalidSnapshot(_) => "invalid snapshot",
        SessionError::IncompatibleSnapshot(_) => "incompatible snapshot",
        SessionError::Backend(_) => "backend operation failed",
        SessionError::Trace(_) => "trace operation failed",
        SessionError::NotInitialized => "storage root is not initialized",
        SessionError::NotAStorageRoot(_) => "not a ChronoFork storage root",
        SessionError::RootNotEmpty(_) => "storage root is not empty",
        SessionError::UnsupportedLayoutVersion { .. } => "unsupported storage layout version",
        SessionError::CorruptMetadata(_) => "corrupt storage metadata",
        SessionError::UnsupportedMetadataVersion { .. } => "unsupported storage metadata version",
        SessionError::MetadataMismatch(_) => "storage metadata mismatch",
        SessionError::IdentityMismatch(_) => "storage identity mismatch",
        SessionError::MissingRecord(_) => "missing storage record",
        SessionError::UnknownStorageSession(_) => "unknown durable session",
        SessionError::UnknownStorageSnapshot { .. } => "unknown durable snapshot",
        SessionError::SessionConflict(_) => "durable session key already exists",
        SessionError::SnapshotExists { .. } => "durable snapshot key already exists",
        SessionError::SessionNotEmpty(_) => "durable session still contains snapshots",
        SessionError::FileTooLarge { .. } => "file exceeds the size limit",
        SessionError::ListLimitExceeded { .. } => "storage listing exceeds its limit",
        SessionError::InvalidName(_) => "invalid storage name",
        SessionError::PathEscape(_) => "managed path escapes the storage root",
        SessionError::SymlinkRejected(_) => "managed path is a symbolic link",
        SessionError::WrongFileType(_) => "managed path has the wrong file type",
        SessionError::DurabilityUnconfirmed { .. } => "durability unconfirmed",
        SessionError::CorruptAllocationCounter(_) => "corrupt durable session-key counter",
        SessionError::WriterContended { .. } => "writer lock is held",
        SessionError::NotLockOwner { .. } => "writer lock ownership lost",
        SessionError::CorruptLock { .. } => "unreadable lock record",
        SessionError::LockPrimitiveUnsupported { .. } => "lock publication is not atomic",
        SessionError::Io { .. } => "storage operation failed",
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::path::PathBuf;

    use chronofork_backend_api::BackendError;
    use chronofork_core::{SessionId, SnapshotId};
    use chronofork_sessions::{LockIdentity, SessionError, SessionState, StorageAnomalyKind};

    use super::{ErrorCode, MAX_ANOMALY_LABELS, ServiceError, anomaly_label};

    fn mapped(error: SessionError) -> ServiceError {
        ServiceError::from(error)
    }

    fn assert_path_free(message: &str) {
        assert!(!message.contains('/'), "no path separator in {message:?}");
        assert!(!message.contains('\\'), "no path separator in {message:?}");
        assert!(message.len() <= 160, "bounded message: {message:?}");
    }

    #[test]
    fn durable_diagnostics_keep_the_session_layer_wording() {
        let cases = [
            (
                SessionError::UnknownStorageSession(7),
                ErrorCode::NotFound,
                "unknown durable session key: 7",
            ),
            (
                SessionError::UnknownStorageSnapshot {
                    session: 1,
                    snapshot: 9,
                },
                ErrorCode::NotFound,
                "unknown durable snapshot key 9 for durable session 1",
            ),
            (
                SessionError::SessionNotEmpty(3),
                ErrorCode::Conflict,
                "durable session 3 still contains snapshots; delete them explicitly first",
            ),
            (
                SessionError::NotInitialized,
                ErrorCode::Storage,
                "storage root is not initialized",
            ),
            (
                SessionError::RootNotEmpty(PathBuf::from("/tmp/secret-store")),
                ErrorCode::Storage,
                "storage root is not empty",
            ),
            (
                SessionError::InvalidName("name is 4 bytes, limit is 3".to_owned()),
                ErrorCode::InvalidRequest,
                "invalid storage name: name is 4 bytes, limit is 3",
            ),
            (
                SessionError::CorruptMetadata("checksum mismatch".to_owned()),
                ErrorCode::Storage,
                "corrupt storage metadata: checksum mismatch",
            ),
            (
                SessionError::MetadataMismatch("snapshot key".to_owned()),
                ErrorCode::Storage,
                "storage metadata mismatch: snapshot key",
            ),
            (
                SessionError::IncompatibleSnapshot("outer format version 2".to_owned()),
                ErrorCode::Storage,
                "incompatible snapshot: outer format version 2",
            ),
            (
                SessionError::UnsupportedLayoutVersion {
                    found: 2,
                    supported: 1,
                },
                ErrorCode::Storage,
                "unsupported storage layout version 2, expected 1",
            ),
            (
                SessionError::NotAStorageRoot("secret marker detail".to_owned()),
                ErrorCode::Storage,
                "not a ChronoFork storage root",
            ),
            (
                SessionError::UnknownSession(SessionId::from_raw(4)),
                ErrorCode::NotFound,
                "unknown session: session-4",
            ),
            (
                SessionError::Closed(SessionId::from_raw(4)),
                ErrorCode::Closed,
                "session is closed: session-4",
            ),
        ];
        for (error, code, message) in cases {
            let mapped = mapped(error);
            assert_eq!(mapped.code(), code, "{message}");
            assert_eq!(mapped.message(), message);
            assert!(!mapped.is_retryable(), "{message}");
        }
    }

    #[test]
    fn path_bearing_variants_never_transport_a_path_or_detail() {
        let secret = PathBuf::from("/tmp/chronofork-secret-store/sessions/session-1");
        let cases = [
            (
                SessionError::RootNotEmpty(secret.clone()),
                ErrorCode::Storage,
            ),
            (
                SessionError::MissingRecord(secret.clone()),
                ErrorCode::Storage,
            ),
            (
                SessionError::FileTooLarge {
                    path: secret.clone(),
                    limit: 64,
                    found: 128,
                },
                ErrorCode::LimitExceeded,
            ),
            (SessionError::PathEscape(secret.clone()), ErrorCode::Storage),
            (
                SessionError::SymlinkRejected(secret.clone()),
                ErrorCode::Storage,
            ),
            (
                SessionError::WrongFileType(secret.clone()),
                ErrorCode::Storage,
            ),
            (
                SessionError::DurabilityUnconfirmed {
                    operation: "publish snapshot",
                    path: secret.clone(),
                },
                ErrorCode::Storage,
            ),
            (
                SessionError::CorruptLock {
                    path: secret.clone(),
                    message: "partial record".to_owned(),
                },
                ErrorCode::Storage,
            ),
            (
                SessionError::LockPrimitiveUnsupported {
                    path: secret.clone(),
                },
                ErrorCode::Storage,
            ),
            (
                SessionError::Io {
                    operation: "read session metadata",
                    path: secret.clone(),
                    kind: ErrorKind::PermissionDenied,
                },
                ErrorCode::Storage,
            ),
            (
                SessionError::NotAStorageRoot("secret marker detail".to_owned()),
                ErrorCode::Storage,
            ),
        ];
        for (error, code) in cases {
            let mapped = mapped(error);
            assert_eq!(mapped.code(), code);
            assert_path_free(mapped.message());
            assert!(
                !mapped.message().contains("chronofork-secret-store"),
                "no host path: {mapped}"
            );
            assert!(
                !mapped.message().contains("secret marker detail"),
                "no embedded detail: {mapped}"
            );
            assert!(
                !mapped.message().contains("PermissionDenied"),
                "no raw I/O detail: {mapped}"
            );
            assert!(
                !mapped.message().contains("session-1"),
                "no managed path component: {mapped}"
            );
        }
    }

    #[test]
    fn mapped_variants_report_the_documented_wording() {
        assert_eq!(
            mapped(SessionError::FileTooLarge {
                path: PathBuf::from("/tmp/x/checkpoint.cfsnap"),
                limit: 64,
                found: 128,
            })
            .message(),
            "file is 128 bytes, limit is 64"
        );
        assert_eq!(
            mapped(SessionError::DurabilityUnconfirmed {
                operation: "publish snapshot",
                path: PathBuf::from("/tmp/x"),
            })
            .message(),
            "publish snapshot is visible but not durably confirmed"
        );
        assert_eq!(
            mapped(SessionError::CorruptLock {
                path: PathBuf::from("/tmp/x/writer.lock"),
                message: "partial record".to_owned(),
            })
            .message(),
            "unreadable lock record: partial record"
        );
        assert_eq!(
            mapped(SessionError::LockPrimitiveUnsupported {
                path: PathBuf::from("/tmp/x"),
            })
            .message(),
            "filesystem does not support hard links, so lock publication cannot be atomic"
        );
        assert_eq!(
            mapped(SessionError::MissingRecord(PathBuf::from(
                "/tmp/x/session.cfmeta"
            )))
            .message(),
            "missing storage record"
        );
        assert_eq!(
            mapped(SessionError::Io {
                operation: "read",
                path: PathBuf::from("/tmp/x"),
                kind: ErrorKind::NotFound,
            })
            .message(),
            "storage operation failed"
        );
        assert_eq!(
            mapped(SessionError::PathEscape(PathBuf::from("/tmp/x"))).message(),
            "managed path escapes the storage root"
        );
        assert_eq!(
            mapped(SessionError::SymlinkRejected(PathBuf::from("/tmp/x"))).message(),
            "managed path is a symbolic link"
        );
        assert_eq!(
            mapped(SessionError::WrongFileType(PathBuf::from("/tmp/x"))).message(),
            "managed path has the wrong file type"
        );
    }

    #[test]
    fn writer_contention_names_the_session_but_never_the_owner() {
        let owner = LockIdentity {
            owner_token: "owner-token-should-not-leak".to_owned(),
            generation: 9,
        };
        let contended = mapped(SessionError::WriterContended {
            session: Some(4),
            owner: Some(owner.clone()),
        });
        assert_eq!(contended.code(), ErrorCode::Conflict);
        assert!(contended.is_retryable());
        assert_eq!(
            contended.message(),
            "durable session 4 writer lock is held by owner"
        );
        assert!(!contended.message().contains("owner-token-should-not-leak"));

        let allocation = mapped(SessionError::WriterContended {
            session: None,
            owner: Some(owner.clone()),
        });
        assert_eq!(allocation.code(), ErrorCode::Conflict);
        assert!(allocation.is_retryable());
        assert_eq!(
            allocation.message(),
            "storage key-allocation lock is held by owner"
        );
        assert!(!allocation.message().contains("owner-token-should-not-leak"));

        let lost = mapped(SessionError::NotLockOwner {
            session: Some(2),
            expected: owner,
        });
        assert_eq!(lost.code(), ErrorCode::Conflict);
        assert!(lost.is_retryable());
        assert_eq!(
            lost.message(),
            "durable session 2 writer lock ownership was lost"
        );
    }

    #[test]
    fn anomaly_labels_are_short_path_free_and_capped() {
        let metadata = StorageAnomalyKind::CorruptObject(SessionError::CorruptMetadata(
            "checksum mismatch".to_owned(),
        ));
        assert_eq!(anomaly_label(&metadata), "corrupt storage metadata");
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::IncompletePublish),
            "incomplete publish"
        );
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::PendingDeletion),
            "pending deletion"
        );
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::UnexpectedEntry),
            "unexpected entry"
        );
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::LockProtocolRecord),
            "lock protocol record"
        );
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::FutureLayoutVersion(3)),
            "future layout version"
        );
        assert_eq!(
            anomaly_label(&StorageAnomalyKind::InvalidKey),
            "invalid key"
        );

        // More distinct labels than the response cap must still be bounded and sorted.
        let corrupt = [
            StorageAnomalyKind::CorruptObject(SessionError::UnknownSession(SessionId::from_raw(1))),
            StorageAnomalyKind::CorruptObject(SessionError::UnknownSnapshot {
                session: SessionId::from_raw(1),
                snapshot: SnapshotId::from_raw(1),
            }),
            StorageAnomalyKind::CorruptObject(SessionError::Closed(SessionId::from_raw(1))),
            StorageAnomalyKind::CorruptObject(SessionError::NotRunning {
                session: SessionId::from_raw(1),
                state: SessionState::Paused,
            }),
            StorageAnomalyKind::CorruptObject(SessionError::Halted(SessionId::from_raw(1))),
            StorageAnomalyKind::CorruptObject(SessionError::IdSpaceExhausted),
            StorageAnomalyKind::CorruptObject(SessionError::InvalidSnapshot("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::IncompatibleSnapshot("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::Backend(BackendError::Halted)),
            StorageAnomalyKind::CorruptObject(SessionError::NotInitialized),
            StorageAnomalyKind::CorruptObject(SessionError::NotAStorageRoot("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::RootNotEmpty(PathBuf::from("/tmp/x"))),
            StorageAnomalyKind::CorruptObject(SessionError::UnsupportedLayoutVersion {
                found: 2,
                supported: 1,
            }),
            StorageAnomalyKind::CorruptObject(SessionError::CorruptMetadata("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::UnsupportedMetadataVersion {
                found: 2,
                supported: 1,
            }),
            StorageAnomalyKind::CorruptObject(SessionError::MetadataMismatch("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::IdentityMismatch("x".to_owned())),
            StorageAnomalyKind::CorruptObject(SessionError::MissingRecord(PathBuf::from("/tmp/x"))),
            StorageAnomalyKind::CorruptObject(SessionError::UnknownStorageSession(1)),
            StorageAnomalyKind::CorruptObject(SessionError::UnknownStorageSnapshot {
                session: 1,
                snapshot: 1,
            }),
        ];
        let mut labels: Vec<&'static str> = corrupt.iter().map(anomaly_label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(
            labels.len(),
            corrupt.len(),
            "every variant has its own label"
        );
        labels.truncate(MAX_ANOMALY_LABELS);
        assert_eq!(MAX_ANOMALY_LABELS, 16);
        assert_eq!(labels.len(), MAX_ANOMALY_LABELS);
        assert!(labels.windows(2).all(|pair| pair[0] < pair[1]));
        for label in labels {
            assert_path_free(label);
            assert!(!label.is_empty());
        }
    }
}
