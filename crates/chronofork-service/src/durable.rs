//! Durable store operations over [`SnapshotRepository`].
//!
//! These functions mirror the `chronofork session save|load|list|delete` workflow of
//! architecture §3.12–§3.15: the repository owns serialization, validation, metadata
//! and locking; this module only orchestrates its public API and maps its typed
//! failures onto the service taxonomy. No host path and no advisory host timestamp is
//! ever transported.
//!
//! `store.load` and `store.save` rehydrate into a private [`SessionManager`], so a
//! durable workflow never allocates or perturbs a live in-memory session identifier.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_core::{InputFrame, SessionId, StateHash, Tick};
use chronofork_sessions::{
    DurableLineage, SessionEntry, SessionManager, SnapshotEntry, SnapshotRepository, StorageAnomaly,
};

use crate::error::{ErrorCode, MAX_ANOMALY_LABELS, ServiceError, anomaly_label};
use crate::json::JsonValue;
use crate::limits::ServiceLimits;
use crate::service::OperationContext;
use crate::values::{optional_name, optional_unsigned, unsigned};

/// Lists durable sessions and their snapshots, bounded by the configured entry cap.
pub(crate) fn list(
    root: &Path,
    limits: &ServiceLimits,
    session_key: Option<u64>,
) -> Result<JsonValue, ServiceError> {
    let repository = SnapshotRepository::open(root).map_err(ServiceError::from)?;
    let mut anomaly_count = 0_usize;
    let mut anomaly_labels: BTreeSet<&'static str> = BTreeSet::new();
    let sessions: Vec<SessionEntry> = match session_key {
        Some(key) => vec![repository.session(key).map_err(ServiceError::from)?],
        None => {
            let listing = repository.list_sessions().map_err(ServiceError::from)?;
            collect_anomalies(&listing.anomalies, &mut anomaly_count, &mut anomaly_labels);
            listing.entries
        }
    };
    let mut budget = limits.max_store_entries;
    let mut truncated = false;
    let mut rendered = Vec::new();
    for session in sessions {
        if budget == 0 {
            truncated = true;
            break;
        }
        budget -= 1;
        let listing = repository
            .list_snapshots(session.session_key)
            .map_err(ServiceError::from)?;
        collect_anomalies(&listing.anomalies, &mut anomaly_count, &mut anomaly_labels);
        let mut snapshots = Vec::new();
        for snapshot in listing.entries {
            if budget == 0 {
                truncated = true;
                break;
            }
            budget -= 1;
            snapshots.push(snapshot_json(&snapshot)?);
        }
        rendered.push(JsonValue::object([
            ("session_key", unsigned(session.session_key)?),
            ("name", optional_name(session.name.as_deref())),
            (
                "parent_session_key",
                optional_unsigned(session.parent_session_key)?,
            ),
            (
                "origin_snapshot_key",
                optional_unsigned(session.origin_snapshot_key)?,
            ),
            ("snapshots", JsonValue::Array(snapshots)),
        ]));
    }
    // The count is the true total; the labels are distinct, sorted, path-free
    // categories bounded by the response cap so a damaged store cannot inflate the
    // listing.
    let labels: Vec<JsonValue> = anomaly_labels
        .into_iter()
        .take(MAX_ANOMALY_LABELS)
        .map(JsonValue::string)
        .collect();
    Ok(JsonValue::object([
        ("sessions", JsonValue::Array(rendered)),
        (
            "anomalies",
            JsonValue::object([
                ("count", unsigned(anomalies_to_u64(anomaly_count))?),
                ("labels", JsonValue::Array(labels)),
            ]),
        ),
        ("truncated", JsonValue::from_bool(truncated)),
    ]))
}

/// Rehydrates one committed checkpoint into a private manager and reports it.
pub(crate) fn load(
    root: &Path,
    factory: &Arc<dyn BackendFactory>,
    session_key: u64,
    snapshot_key: Option<u64>,
    steps: u64,
    buttons: u32,
) -> Result<JsonValue, ServiceError> {
    let repository = SnapshotRepository::open(root).map_err(ServiceError::from)?;
    let snapshot_key = match snapshot_key {
        Some(key) => key,
        None => {
            repository
                .latest_snapshot(session_key)
                .map_err(ServiceError::from)?
                .ok_or_else(missing_committed_snapshot)?
                .snapshot_key
        }
    };
    let loaded = repository
        .load_snapshot(session_key, snapshot_key)
        .map_err(ServiceError::from)?;
    let mut manager = SessionManager::new(Arc::clone(factory));
    let runtime = manager
        .create_session_from_stored_snapshot(loaded.snapshot, session_key)
        .map_err(ServiceError::from)?;
    advance(&mut manager, runtime, steps, buttons)?;
    let view = manager.inspect(runtime).map_err(ServiceError::from)?;
    let state_hash = manager.state_hash(runtime).map_err(ServiceError::from)?;
    report(session_key, &loaded.entry, view.machine.tick, state_hash)
}

/// Creates a durable session, or continues an existing one, and publishes a checkpoint.
///
/// `context` is the caller's operation context: the durable write that first becomes
/// visible is the mutation's first irreversible effect, so this function marks it with
/// `context.begin()` **before** it happens. A failure after that point (a checkpoint
/// write, a lock release) therefore keeps the journal entry and a retry with the same
/// key replays the recorded outcome instead of publishing a second session.
pub(crate) fn save(
    root: &Path,
    factory: &Arc<dyn BackendFactory>,
    session_key: Option<u64>,
    steps: u64,
    buttons: u32,
    name: Option<&str>,
    context: &OperationContext<'_>,
) -> Result<JsonValue, ServiceError> {
    match session_key {
        Some(session_key) => {
            save_continuation(root, factory, session_key, steps, buttons, name, context)
        }
        None => save_new(root, factory, steps, buttons, name, context),
    }
}

/// Publishes a new durable session and its first checkpoint.
///
/// All fallible work — advancing the machine, capturing the state and serializing the
/// record — happens **before** the first durable write, so a backend or quota failure
/// leaves no partially published session behind. Only the last two steps touch the
/// store, and the first of them is journaled.
fn save_new(
    root: &Path,
    factory: &Arc<dyn BackendFactory>,
    steps: u64,
    buttons: u32,
    name: Option<&str>,
    context: &OperationContext<'_>,
) -> Result<JsonValue, ServiceError> {
    // A brand-new durable session may initialize an empty root; continuing an
    // existing session never creates one as a side effect of a command about to fail.
    // An initialized-but-empty root carries no session and no checkpoint, so it is not a
    // published mutation.
    let repository = SnapshotRepository::initialize(root).map_err(ServiceError::from)?;
    let mut manager = SessionManager::new(Arc::clone(factory));
    let runtime = manager.create_session().map_err(ServiceError::from)?;
    advance(&mut manager, runtime, steps, buttons)?;
    let snapshot = manager.snapshot(runtime).map_err(ServiceError::from)?;
    let record = manager
        .snapshot_record(runtime, snapshot)
        .map_err(ServiceError::from)?;
    // Everything that can fail before the store is written has already happened; this is
    // the mutation's first irreversible effect.
    context.begin();
    let session = repository
        .create_session(
            SessionId::from_raw(runtime.raw()),
            name,
            DurableLineage::root(),
        )
        .map_err(ServiceError::from)?;
    manager
        .attach_storage_session(runtime, session.session_key)
        .map_err(ServiceError::from)?;
    let entry = repository
        .save_snapshot(session.session_key, &record, name)
        .map_err(ServiceError::from)?;
    report(session.session_key, &entry, entry.tick, entry.state_hash)
}

fn save_continuation(
    root: &Path,
    factory: &Arc<dyn BackendFactory>,
    session_key: u64,
    steps: u64,
    buttons: u32,
    name: Option<&str>,
    context: &OperationContext<'_>,
) -> Result<JsonValue, ServiceError> {
    // Continue-and-save is one linearizable read-modify-publish workflow: the
    // per-session writer lock is taken before the base is read and held through
    // rehydration, advance, capture and publish. A concurrent writer fails with a
    // typed retryable conflict instead of reading a stale base. The only durable write
    // is the final publish, and it is the mutation's first irreversible effect, so it
    // is journaled immediately before it happens.
    let repository = SnapshotRepository::open(root).map_err(ServiceError::from)?;
    let entry = repository.with_writer_lock(
        session_key,
        |repository, lock| -> Result<SnapshotEntry, ServiceError> {
            let latest = repository
                .latest_snapshot(session_key)
                .map_err(ServiceError::from)?
                .ok_or_else(missing_committed_snapshot)?;
            let loaded = repository
                .load_snapshot(session_key, latest.snapshot_key)
                .map_err(ServiceError::from)?;
            let mut manager = SessionManager::new(Arc::clone(factory));
            let runtime = manager
                .create_session_from_stored_snapshot(loaded.snapshot, session_key)
                .map_err(ServiceError::from)?;
            advance(&mut manager, runtime, steps, buttons)?;
            let snapshot = manager.snapshot(runtime).map_err(ServiceError::from)?;
            let record = manager
                .snapshot_record(runtime, snapshot)
                .map_err(ServiceError::from)?;
            context.begin();
            repository
                .save_snapshot_with_lock(lock, &record, name)
                .map_err(ServiceError::from)
        },
    )?;
    report(session_key, &entry, entry.tick, entry.state_hash)
}

/// Deletes one durable snapshot, or one empty durable session.
pub(crate) fn delete(
    root: &Path,
    session_key: u64,
    snapshot_key: Option<u64>,
) -> Result<JsonValue, ServiceError> {
    let repository = SnapshotRepository::open(root).map_err(ServiceError::from)?;
    match snapshot_key {
        Some(snapshot_key) => repository
            .delete_snapshot(session_key, snapshot_key)
            .map_err(ServiceError::from)?,
        None => repository
            .delete_session(session_key)
            .map_err(ServiceError::from)?,
    }
    Ok(JsonValue::object([
        ("session_key", unsigned(session_key)?),
        ("snapshot_key", optional_unsigned(snapshot_key)?),
    ]))
}

fn report(
    session_key: u64,
    entry: &SnapshotEntry,
    tick: Tick,
    state_hash: StateHash,
) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        ("session_key", unsigned(session_key)?),
        ("snapshot_key", unsigned(entry.snapshot_key)?),
        ("name", optional_name(entry.name.as_deref())),
        ("creation_sequence", unsigned(entry.creation_sequence)?),
        ("tick", unsigned(tick.raw())?),
        ("state_hash", JsonValue::string(state_hash.to_hex())),
        (
            "parent_session_key",
            optional_unsigned(entry.parent_session_key)?,
        ),
        (
            "origin_snapshot_key",
            optional_unsigned(entry.origin_snapshot_key)?,
        ),
    ]))
}

fn snapshot_json(entry: &SnapshotEntry) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        ("snapshot_key", unsigned(entry.snapshot_key)?),
        ("name", optional_name(entry.name.as_deref())),
        ("creation_sequence", unsigned(entry.creation_sequence)?),
        ("tick", unsigned(entry.tick.raw())?),
        ("state_hash", JsonValue::string(entry.state_hash.to_hex())),
        ("checkpoint_length", unsigned(entry.checkpoint_length)?),
        (
            "checkpoint_sha256",
            JsonValue::string(entry.checkpoint_sha256.to_hex()),
        ),
    ]))
}

fn advance(
    manager: &mut SessionManager,
    session: SessionId,
    steps: u64,
    buttons: u32,
) -> Result<(), ServiceError> {
    let input = InputFrame::new(buttons);
    for _ in 0..steps {
        manager.step(session, input).map_err(ServiceError::from)?;
    }
    Ok(())
}

/// Accumulates the anomaly total and its bounded, deterministic label set.
fn collect_anomalies(
    anomalies: &[StorageAnomaly],
    count: &mut usize,
    labels: &mut BTreeSet<&'static str>,
) {
    *count = count.saturating_add(anomalies.len());
    for anomaly in anomalies {
        labels.insert(anomaly_label(&anomaly.kind));
    }
}

fn missing_committed_snapshot() -> ServiceError {
    ServiceError::new(
        ErrorCode::NotFound,
        "durable session has no committed snapshot",
    )
}

fn anomalies_to_u64(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}
