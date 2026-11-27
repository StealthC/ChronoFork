//! Deterministic, content-addressed experiment artifacts.
//!
//! An artifact is a canonical document, not emulated state: the session layer stays
//! the only canonical owner. It contains no host time, no path and no principal, so
//! the same request produces byte-identical bytes on every surface and run. See
//! architecture §3.16 and §9.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chronofork_backend_api::BackendFactory;
use chronofork_core::{InputFrame, hash_bytes};
use chronofork_sessions::SessionManager;

use crate::error::{ErrorCode, ServiceError};
use crate::json::JsonValue;
use crate::limits::ServiceLimits;
use crate::values::unsigned;

/// Suffix of one persisted artifact document.
const ARTIFACT_SUFFIX: &str = ".cfexp.json";

/// Monotonic counter that keeps concurrent temporary names distinct.
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One produced artifact and its persisted bytes.
pub(crate) struct Experiment {
    /// The canonical `chronofork.experiment.v1` document.
    pub(crate) artifact: JsonValue,
    /// SHA-256 of the canonical artifact bytes.
    pub(crate) artifact_id: String,
    /// The exact canonical bytes whose digest is `artifact_id`.
    pub(crate) bytes: Vec<u8>,
}

/// Runs one bounded experiment on a private session manager.
///
/// The private manager is never registered with the service, so an experiment can
/// not change a live session identifier or its state.
pub(crate) fn run(
    factory: &Arc<dyn BackendFactory>,
    limits: &ServiceLimits,
    steps: u64,
    buttons: u32,
    checkpoints: &[u64],
) -> Result<Experiment, ServiceError> {
    validate_checkpoints(limits, steps, checkpoints)?;
    let descriptor = factory.descriptor().clone();
    let mut manager = SessionManager::new(Arc::clone(factory));
    let session = manager.create_session().map_err(ServiceError::from)?;
    // The configuration hash is read from a fresh session's initial snapshot
    // envelope: it identifies the immutable machine configuration, so it is the same
    // value the session layer will record in every checkpoint of this experiment.
    let initial = manager.snapshot(session).map_err(ServiceError::from)?;
    let config_hash = manager
        .snapshot_record(session, initial)
        .map_err(ServiceError::from)?
        .backend()
        .config_hash()
        .to_hex();
    let view = manager.inspect(session).map_err(ServiceError::from)?;
    let mut tick = view.machine.tick.raw();
    let mut state_hash = view.machine.state_hash;

    let input = InputFrame::new(buttons);
    let mut captured: Vec<JsonValue> = Vec::new();
    let mut pending = checkpoints.iter().peekable();
    let mut performed = 0_u64;
    let mut halted = false;
    loop {
        while pending
            .peek()
            .is_some_and(|checkpoint| **checkpoint == performed)
        {
            captured.push(checkpoint_json(performed, tick, state_hash)?);
            pending.next();
        }
        if performed == steps || halted {
            break;
        }
        // The artifact records a per-step digest, so the verified path supplies it
        // instead of the ordinary outcome's migration-only field.
        let outcome = manager
            .step_verified(session, input)
            .map_err(ServiceError::from)?;
        performed = performed
            .checked_add(1)
            .ok_or_else(|| ServiceError::internal("step counter overflow"))?;
        tick = outcome.tick.raw();
        state_hash = outcome.state_hash;
        halted = outcome.halted;
    }

    let artifact = JsonValue::object([
        ("schema", JsonValue::string("chronofork.experiment.v1")),
        (
            "backend",
            JsonValue::object([
                ("id", JsonValue::string(descriptor.id().as_str())),
                ("display_name", JsonValue::string(descriptor.display_name())),
                ("version", JsonValue::string(descriptor.version())),
                ("config_hash", JsonValue::string(config_hash)),
                (
                    "test_backend",
                    JsonValue::from_bool(descriptor.is_test_backend()),
                ),
            ]),
        ),
        (
            "input",
            JsonValue::object([
                ("steps", unsigned(steps)?),
                ("buttons", unsigned(u64::from(buttons))?),
            ]),
        ),
        ("checkpoints", JsonValue::Array(captured)),
        (
            "final",
            JsonValue::object([
                ("step", unsigned(performed)?),
                ("tick", unsigned(tick)?),
                ("state_hash", JsonValue::string(state_hash.to_hex())),
                ("halted", JsonValue::from_bool(halted)),
            ]),
        ),
    ]);
    let bytes = crate::json::to_canonical_bytes(&artifact, limits)?;
    let artifact_id = hash_bytes(&bytes).to_hex();
    Ok(Experiment {
        artifact,
        artifact_id,
        bytes,
    })
}

/// Writes `<artifact_id>.cfexp.json` atomically under `root` unless it already exists.
///
/// Publication is a temporary file plus a rename, so a reader never observes a
/// partial document. An identifier that already exists is never reused with
/// different bytes.
pub(crate) fn persist(root: &Path, artifact_id: &str, bytes: &[u8]) -> Result<(), ServiceError> {
    fs::create_dir_all(root)
        .map_err(|_| ServiceError::new(ErrorCode::Storage, "artifact root is not writable"))?;
    let path = root.join(format!("{artifact_id}{ARTIFACT_SUFFIX}"));
    if path.exists() {
        let existing = fs::read(&path).map_err(|_| {
            ServiceError::new(ErrorCode::Storage, "existing artifact could not be read")
        })?;
        if existing == bytes {
            return Ok(());
        }
        return Err(ServiceError::new(
            ErrorCode::Storage,
            "artifact identifier already exists with different bytes",
        ));
    }
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = root.join(format!(
        ".{artifact_id}{ARTIFACT_SUFFIX}.tmp-{}-{sequence}",
        std::process::id()
    ));
    if let Err(error) = write_and_sync(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if fs::rename(&temporary, &path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(ServiceError::new(
            ErrorCode::Storage,
            "artifact could not be published",
        ));
    }
    Ok(())
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    let mut file = File::create(path)
        .map_err(|_| ServiceError::new(ErrorCode::Storage, "artifact could not be created"))?;
    file.write_all(bytes)
        .map_err(|_| ServiceError::new(ErrorCode::Storage, "artifact could not be written"))?;
    file.sync_all()
        .map_err(|_| ServiceError::new(ErrorCode::Storage, "artifact could not be flushed"))
}

fn checkpoint_json(
    step: u64,
    tick: u64,
    state_hash: chronofork_core::StateHash,
) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        ("step", unsigned(step)?),
        ("tick", unsigned(tick)?),
        ("state_hash", JsonValue::string(state_hash.to_hex())),
    ]))
}

fn validate_checkpoints(
    limits: &ServiceLimits,
    steps: u64,
    checkpoints: &[u64],
) -> Result<(), ServiceError> {
    if checkpoints.len() > limits.max_checkpoints {
        return Err(ServiceError::new(
            ErrorCode::LimitExceeded,
            "`checkpoints` exceeds the configured checkpoint limit",
        ));
    }
    let mut previous: Option<u64> = None;
    for checkpoint in checkpoints {
        if *checkpoint > steps {
            return Err(ServiceError::new(
                ErrorCode::InvalidRequest,
                "`checkpoints` entries must be at most `steps`",
            ));
        }
        if previous.is_some_and(|previous| *checkpoint <= previous) {
            return Err(ServiceError::new(
                ErrorCode::InvalidRequest,
                "`checkpoints` must be strictly increasing",
            ));
        }
        previous = Some(*checkpoint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_checkpoints;
    use crate::error::ErrorCode;
    use crate::limits::ServiceLimits;

    fn code(limits: &ServiceLimits, steps: u64, checkpoints: &[u64]) -> ErrorCode {
        validate_checkpoints(limits, steps, checkpoints)
            .expect_err("checkpoints must be rejected")
            .code()
    }

    #[test]
    fn valid_checkpoint_lists_are_accepted() {
        let limits = ServiceLimits::default();
        validate_checkpoints(&limits, 10, &[]).expect("empty");
        validate_checkpoints(&limits, 10, &[0]).expect("zero");
        validate_checkpoints(&limits, 10, &[0, 5, 10]).expect("all");
    }

    #[test]
    fn invalid_checkpoint_lists_are_typed() {
        let limits = ServiceLimits::default();
        assert_eq!(code(&limits, 10, &[11]), ErrorCode::InvalidRequest);
        assert_eq!(code(&limits, 10, &[5, 5]), ErrorCode::InvalidRequest);
        assert_eq!(code(&limits, 10, &[5, 4]), ErrorCode::InvalidRequest);
        let limits = ServiceLimits {
            max_checkpoints: 2,
            ..ServiceLimits::default()
        };
        assert_eq!(code(&limits, 10, &[0, 1, 2]), ErrorCode::LimitExceeded);
    }
}
