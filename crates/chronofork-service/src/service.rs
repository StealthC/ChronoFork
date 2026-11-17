//! The application service: operation dispatch, ownership and synchronization.
//!
//! `SessionManager` stays synchronous and lock-free. This type is the single
//! documented synchronization boundary: one manager lock, one writer lease per
//! session and immutable principals. See ADR-0012 decision 5 and architecture §7.
//!
//! Lock discipline, in acquisition order:
//!
//! 1. the per-session writer lease table (`leases`), held only for a lookup/insert or
//!    a removal, so a second mutation of the same session fails immediately instead of
//!    blocking on the manager lock;
//! 2. the session manager (`manager`), held for the duration of one operation and
//!    never across socket I/O;
//! 3. the ownership registry (`registry`), held briefly and only while the manager
//!    lock is already held when both are needed.
//!
//! Reads never take a writer lease, so they are never rejected with `conflict`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chronofork_backend_api::{BackendDescriptor, BackendFactory};
use chronofork_core::{InputFrame, SessionId, SnapshotId, hash_bytes};
use chronofork_sessions::{Lineage, SessionError, SessionManager, SessionState};

use crate::auth::Principal;
use crate::catalog::{Args, CATALOG, OpKind, OpSpec};
use crate::error::{ErrorCode, ServiceError};
use crate::json::JsonValue;
use crate::limits::ServiceLimits;
use crate::tool::ToolSpec;
use crate::values::{descriptor_json, hex, optional_name, optional_unsigned, unsigned};

/// Version reported by `capabilities` and MCP `serverInfo`.
pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Configuration for one service instance.
pub struct ServiceConfig {
    /// Backend family used for every session this instance creates.
    pub factory: Arc<dyn BackendFactory>,
    /// Optional durable storage root.
    pub storage_root: Option<PathBuf>,
    /// Optional content-addressed experiment artifact root.
    pub artifacts_root: Option<PathBuf>,
    /// Resource limits applied to every operation.
    pub limits: ServiceLimits,
}

impl fmt::Debug for ServiceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceConfig")
            .field("backend", self.factory.descriptor().id())
            .field("storage_root", &self.storage_root)
            .field("artifacts_root", &self.artifacts_root)
            .field("limits", &self.limits)
            .finish()
    }
}

/// Service-level metadata for one registered session.
#[derive(Clone, Debug)]
struct SessionRecord {
    /// Principal name recorded when the session was created.
    owner: String,
    /// Whether `session.close` has been applied.
    closed: bool,
}

/// The ownership registry: monotonic session metadata, never session state.
#[derive(Debug, Default)]
struct Registry {
    records: BTreeMap<SessionId, SessionRecord>,
}

impl Registry {
    fn add(&mut self, session: SessionId, owner: &str) {
        self.records.insert(
            session,
            SessionRecord {
                owner: owner.to_owned(),
                closed: false,
            },
        );
    }

    /// Number of registered sessions, including closed ones.
    ///
    /// `max_sessions` is a bound on this count: the registry has no removal API, so a
    /// closed session still occupies a registration slot for the process lifetime.
    fn registered_count(&self) -> usize {
        self.records.len()
    }

    /// Number of sessions that are not closed, reported by [`AppService::session_count`].
    fn live_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| !record.closed)
            .count()
    }
}

/// Active per-session writer leases.
#[derive(Debug, Default)]
struct LeaseTable {
    held: BTreeMap<SessionId, u64>,
    next: u64,
}

/// One in-process application service.
pub struct AppService {
    manager: Mutex<SessionManager>,
    registry: Mutex<Registry>,
    leases: Mutex<LeaseTable>,
    factory: Arc<dyn BackendFactory>,
    storage_root: Option<PathBuf>,
    artifacts_root: Option<PathBuf>,
    limits: ServiceLimits,
}

impl fmt::Debug for AppService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppService")
            .field("backend", self.backend().id())
            .field("storage_root", &self.storage_root)
            .field("artifacts_root", &self.artifacts_root)
            .field("limits", &self.limits)
            .finish()
    }
}

/// A held writer lease that is released when it goes out of scope.
///
/// The lease is declared before the manager guard so it is released after it: a
/// waiting mutation can only observe a free lease once the manager is free again.
struct Lease<'a> {
    service: &'a AppService,
    session: SessionId,
    token: u64,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.service.leases.lock() {
            if leases.held.get(&self.session) == Some(&self.token) {
                leases.held.remove(&self.session);
            }
        }
    }
}

impl AppService {
    /// Creates a service over the configured backend factory.
    ///
    /// # Errors
    ///
    /// Currently infallible; the signature is frozen by ADR-0012 so a future
    /// validation step can be added without a breaking change.
    pub fn new(config: ServiceConfig) -> Result<Self, ServiceError> {
        let manager = SessionManager::new(Arc::clone(&config.factory));
        Ok(Self {
            manager: Mutex::new(manager),
            registry: Mutex::new(Registry::default()),
            leases: Mutex::new(LeaseTable::default()),
            factory: config.factory,
            storage_root: config.storage_root,
            artifacts_root: config.artifacts_root,
            limits: config.limits,
        })
    }

    /// Returns the backend descriptor for this instance.
    #[must_use]
    pub fn backend(&self) -> &BackendDescriptor {
        self.factory.descriptor()
    }

    /// Returns the configured storage root, when durable operations are enabled.
    #[must_use]
    pub fn storage_root(&self) -> Option<&Path> {
        self.storage_root.as_deref()
    }

    /// Returns the configured artifact root, when artifacts are persisted.
    #[must_use]
    pub fn artifacts_root(&self) -> Option<&Path> {
        self.artifacts_root.as_deref()
    }

    /// Returns the applied resource limits.
    #[must_use]
    pub const fn limits(&self) -> &ServiceLimits {
        &self.limits
    }

    /// Returns the number of live in-memory sessions.
    ///
    /// Closed sessions stay registered in the session layer but are not live and are
    /// not counted.
    pub fn session_count(&self) -> Result<usize, ServiceError> {
        Ok(self.lock_registry()?.live_count())
    }

    /// Returns the catalog of machine-callable operations.
    #[must_use]
    pub fn tools(&self) -> Vec<ToolSpec> {
        crate::catalog::tools(&self.limits)
    }

    /// Dispatches one canonical request document from an authenticated principal.
    ///
    /// The request document is `{"op": <name>, "input": {<fields>}}`. A successful call
    /// returns the canonical bytes of `{"op": <name>, "result": {<fields>}}`; every
    /// failure returns a typed [`ServiceError`] whose [`ServiceError::to_canonical_bytes`]
    /// form is the `chronofork.error.v1` envelope. The transport maps the error code to a
    /// status and never rewrites a result field.
    ///
    /// # Errors
    ///
    /// Every documented `ErrorCode` can be returned; no error message contains a host
    /// path, ROM bytes or token material.
    pub fn dispatch(&self, principal: &Principal, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        let document = crate::json::parse(request, &self.limits)?;
        let (op, input) = request_parts(&document)?;
        let spec = crate::catalog::find(op)
            .ok_or_else(|| ServiceError::new(ErrorCode::UnknownOp, "unknown operation"))?;
        if !principal.has_scope(spec.scope) {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                format!("principal lacks the `{}` scope", spec.scope.as_str()),
            ));
        }
        let args = crate::catalog::validate(spec, input, &self.limits)?;
        let result = self.execute(spec, principal, &args)?;
        let envelope =
            JsonValue::object([("op", JsonValue::string(spec.name)), ("result", result)]);
        crate::json::to_canonical_bytes(&envelope, &self.limits)
    }

    fn execute(
        &self,
        spec: &OpSpec,
        principal: &Principal,
        args: &Args,
    ) -> Result<JsonValue, ServiceError> {
        match spec.kind {
            OpKind::Capabilities => self.capabilities(),
            OpKind::Create => self.create(principal),
            OpKind::List => self.list(principal),
            OpKind::Inspect => {
                let session = self.authorized_session(principal, args)?;
                self.inspect(session)
            }
            OpKind::Run => self.run(principal, args),
            OpKind::Fork => self.fork(principal, args),
            OpKind::Snapshot => self.snapshot(principal, args),
            OpKind::Restore => self.restore(principal, args),
            OpKind::Compare => self.compare(principal, args),
            OpKind::Close => self.close(principal, args),
            OpKind::ReadMemory => self.read_memory(principal, args),
            OpKind::StoreList => {
                let root = self.require_storage_root()?;
                crate::durable::list(root, &self.limits, args.optional_unsigned("session_key")?)
            }
            OpKind::StoreLoad => {
                let root = self.require_storage_root()?;
                crate::durable::load(
                    root,
                    &self.factory,
                    args.unsigned("session_key")?,
                    args.optional_unsigned("snapshot_key")?,
                    args.unsigned_or_zero("steps")?,
                    args.buttons()?,
                )
            }
            OpKind::StoreSave => {
                let root = self.require_storage_root()?;
                crate::durable::save(
                    root,
                    &self.factory,
                    args.optional_unsigned("session_key")?,
                    args.unsigned_or_zero("steps")?,
                    args.buttons()?,
                    args.name("name"),
                )
            }
            OpKind::StoreDelete => {
                let root = self.require_storage_root()?;
                crate::durable::delete(
                    root,
                    args.unsigned("session_key")?,
                    args.optional_unsigned("snapshot_key")?,
                )
            }
            OpKind::ExperimentRun => self.experiment(args),
        }
    }

    fn capabilities(&self) -> Result<JsonValue, ServiceError> {
        let ops: Vec<JsonValue> = CATALOG
            .iter()
            .map(|spec| {
                JsonValue::object([
                    ("name", JsonValue::string(spec.name)),
                    ("mutating", JsonValue::from_bool(spec.mutating)),
                ])
            })
            .collect();
        let limits = self.limits_json()?;
        Ok(JsonValue::object([
            ("schema", JsonValue::string("chronofork.capabilities.v1")),
            ("service_version", JsonValue::string(SERVICE_VERSION)),
            ("backend", descriptor_json(self.backend())),
            ("ops", JsonValue::Array(ops)),
            ("limits", limits),
            (
                "store",
                JsonValue::object([(
                    "configured",
                    JsonValue::from_bool(self.storage_root.is_some()),
                )]),
            ),
            (
                "artifacts",
                JsonValue::object([(
                    "configured",
                    JsonValue::from_bool(self.artifacts_root.is_some()),
                )]),
            ),
        ]))
    }

    fn limits_json(&self) -> Result<JsonValue, ServiceError> {
        let limits = self.limits;
        Ok(JsonValue::object([
            (
                "max_request_bytes",
                unsigned(bytes_to_u64(limits.max_request_bytes))?,
            ),
            (
                "max_response_bytes",
                unsigned(bytes_to_u64(limits.max_response_bytes))?,
            ),
            (
                "max_steps_per_request",
                unsigned(limits.max_steps_per_request)?,
            ),
            (
                "max_memory_read",
                unsigned(bytes_to_u64(limits.max_memory_read))?,
            ),
            ("max_sessions", unsigned(bytes_to_u64(limits.max_sessions))?),
            (
                "max_store_entries",
                unsigned(bytes_to_u64(limits.max_store_entries))?,
            ),
            (
                "max_checkpoints",
                unsigned(bytes_to_u64(limits.max_checkpoints))?,
            ),
            (
                "json_max_depth",
                unsigned(bytes_to_u64(limits.json_max_depth))?,
            ),
        ]))
    }

    fn create(&self, principal: &Principal) -> Result<JsonValue, ServiceError> {
        let mut manager = self.lock_manager()?;
        let mut registry = self.lock_registry()?;
        if registry.registered_count() >= self.limits.max_sessions {
            return Err(session_limit());
        }
        let session = manager.create_session().map_err(ServiceError::from)?;
        registry.add(session, principal.name());
        let view = manager.inspect(session).map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string(state_name(view.state))),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("lineage", lineage_json(&view.lineage)?),
            ("backend_id", JsonValue::string(view.backend.id().as_str())),
        ]))
    }

    fn list(&self, principal: &Principal) -> Result<JsonValue, ServiceError> {
        let manager = self.lock_manager()?;
        let registry = self.lock_registry()?;
        let mut sessions = Vec::new();
        let mut truncated = false;
        for (session, record) in &registry.records {
            if record.closed {
                continue;
            }
            if !principal.is_admin() && record.owner != principal.name() {
                continue;
            }
            if sessions.len() >= self.limits.max_store_entries {
                truncated = true;
                break;
            }
            match manager.inspect(*session) {
                Ok(view) => sessions.push(JsonValue::object([
                    ("session", unsigned(session.raw())?),
                    ("state", JsonValue::string(state_name(view.state))),
                    ("tick", unsigned(view.machine.tick.raw())?),
                    (
                        "state_hash",
                        JsonValue::string(view.machine.state_hash.to_hex()),
                    ),
                    ("lineage", lineage_json(&view.lineage)?),
                ])),
                Err(SessionError::Closed(_)) => {}
                Err(error) => return Err(ServiceError::from(error)),
            }
        }
        Ok(JsonValue::object([
            ("sessions", JsonValue::Array(sessions)),
            ("truncated", JsonValue::from_bool(truncated)),
        ]))
    }

    fn inspect(&self, session: SessionId) -> Result<JsonValue, ServiceError> {
        let manager = self.lock_manager()?;
        let view = manager.inspect(session).map_err(ServiceError::from)?;
        let mut registers = BTreeMap::new();
        for (name, value) in &view.machine.registers {
            registers.insert(name.clone(), unsigned(*value)?);
        }
        let machine = JsonValue::object([
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("halted", JsonValue::from_bool(view.machine.halted)),
            ("registers", JsonValue::Object(registers)),
            ("memory_base", unsigned(view.machine.memory_base)?),
            (
                "memory_length",
                unsigned(bytes_to_u64(view.machine.memory.len()))?,
            ),
        ]);
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string(state_name(view.state))),
            ("lineage", lineage_json(&view.lineage)?),
            ("backend", descriptor_json(&view.backend)),
            ("machine", machine),
        ]))
    }

    fn run(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let steps = args.unsigned("steps")?;
        let buttons = args.buttons()?;
        let _lease = self.acquire_lease(session)?;
        let mut manager = self.lock_manager()?;
        let input = InputFrame::new(buttons);
        let mut performed = 0_u64;
        let mut halted = false;
        while performed < steps {
            let outcome = manager.step(session, input).map_err(ServiceError::from)?;
            performed = performed
                .checked_add(1)
                .ok_or_else(|| ServiceError::internal("step counter overflow"))?;
            halted = outcome.halted;
            if halted {
                break;
            }
        }
        let view = manager.inspect(session).map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("steps", unsigned(performed)?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("halted", JsonValue::from_bool(halted)),
        ]))
    }

    fn fork(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let parent = self.authorized_session(principal, args)?;
        let _lease = self.acquire_lease(parent)?;
        let mut manager = self.lock_manager()?;
        let mut registry = self.lock_registry()?;
        if registry.registered_count() >= self.limits.max_sessions {
            return Err(session_limit());
        }
        let child = manager.fork_current(parent).map_err(ServiceError::from)?;
        registry.add(child, principal.name());
        let view = manager.inspect(child).map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(child.raw())?),
            ("parent", unsigned(parent.raw())?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
            ("lineage", lineage_json(&view.lineage)?),
        ]))
    }

    fn snapshot(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let _lease = self.acquire_lease(session)?;
        let mut manager = self.lock_manager()?;
        let snapshot = manager.snapshot(session).map_err(ServiceError::from)?;
        let record = manager
            .snapshot_record(session, snapshot)
            .map_err(ServiceError::from)?;
        let bytes = record.to_bytes().map_err(ServiceError::from)?;
        let tick = record.tick();
        let state_hash = record.state_hash();
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("snapshot", unsigned(snapshot.raw())?),
            ("name", optional_name(args.name("name"))),
            ("tick", unsigned(tick.raw())?),
            ("state_hash", JsonValue::string(state_hash.to_hex())),
            ("bytes_length", unsigned(bytes_to_u64(bytes.len()))?),
            (
                "bytes_sha256",
                JsonValue::string(hash_bytes(&bytes).to_hex()),
            ),
        ]))
    }

    fn restore(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let snapshot = SnapshotId::from_raw(args.unsigned("snapshot")?);
        let _lease = self.acquire_lease(session)?;
        let mut manager = self.lock_manager()?;
        manager
            .restore(session, snapshot)
            .map_err(ServiceError::from)?;
        let view = manager.inspect(session).map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("snapshot", unsigned(snapshot.raw())?),
            ("tick", unsigned(view.machine.tick.raw())?),
            (
                "state_hash",
                JsonValue::string(view.machine.state_hash.to_hex()),
            ),
        ]))
    }

    fn compare(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let left = SessionId::from_raw(args.unsigned("left")?);
        let right = SessionId::from_raw(args.unsigned("right")?);
        self.authorize(principal, left)?;
        self.authorize(principal, right)?;
        let manager = self.lock_manager()?;
        let comparison = manager.compare(left, right).map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("left", unsigned(comparison.left.raw())?),
            ("right", unsigned(comparison.right.raw())?),
            ("left_tick", unsigned(comparison.left_tick.raw())?),
            ("right_tick", unsigned(comparison.right_tick.raw())?),
            (
                "left_state_hash",
                JsonValue::string(comparison.left_hash.to_hex()),
            ),
            (
                "right_state_hash",
                JsonValue::string(comparison.right_hash.to_hex()),
            ),
            ("equivalent", JsonValue::from_bool(comparison.equivalent)),
        ]))
    }

    fn close(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let _lease = self.acquire_lease(session)?;
        let mut manager = self.lock_manager()?;
        manager.close(session).map_err(ServiceError::from)?;
        {
            let mut registry = self.lock_registry()?;
            if let Some(record) = registry.records.get_mut(&session) {
                record.closed = true;
            }
        }
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("state", JsonValue::string("closed")),
        ]))
    }

    fn read_memory(&self, principal: &Principal, args: &Args) -> Result<JsonValue, ServiceError> {
        let session = self.authorized_session(principal, args)?;
        let address = args.unsigned("address")?;
        let length = usize::try_from(args.unsigned_or_zero("length")?)
            .map_err(|_| ServiceError::internal("validated memory length is out of range"))?;
        let manager = self.lock_manager()?;
        let bytes = manager
            .read_memory(session, address, length)
            .map_err(ServiceError::from)?;
        Ok(JsonValue::object([
            ("session", unsigned(session.raw())?),
            ("address", unsigned(address)?),
            ("length", unsigned(bytes_to_u64(bytes.len()))?),
            ("bytes", JsonValue::string(hex(&bytes))),
        ]))
    }

    fn experiment(&self, args: &Args) -> Result<JsonValue, ServiceError> {
        let experiment = crate::experiment::run(
            &self.factory,
            &self.limits,
            args.unsigned("steps")?,
            args.buttons()?,
            args.checkpoints(),
        )?;
        if let Some(root) = &self.artifacts_root {
            crate::experiment::persist(root, &experiment.artifact_id, &experiment.bytes)?;
        }
        let crate::experiment::Experiment {
            artifact,
            artifact_id,
            ..
        } = experiment;
        Ok(JsonValue::object([
            ("artifact", artifact),
            ("artifact_id", JsonValue::string(artifact_id)),
        ]))
    }

    /// Resolves the `session` field and enforces per-principal ownership.
    fn authorized_session(
        &self,
        principal: &Principal,
        args: &Args,
    ) -> Result<SessionId, ServiceError> {
        let session = SessionId::from_raw(args.unsigned("session")?);
        self.authorize(principal, session)?;
        Ok(session)
    }

    /// Enforces that the principal may observe and mutate `session`.
    fn authorize(&self, principal: &Principal, session: SessionId) -> Result<(), ServiceError> {
        let registry = self.lock_registry()?;
        let record = registry
            .records
            .get(&session)
            .ok_or_else(|| ServiceError::new(ErrorCode::NotFound, "unknown session"))?;
        if !principal.is_admin() && record.owner != principal.name() {
            return Err(ServiceError::new(
                ErrorCode::Forbidden,
                "session belongs to another principal",
            ));
        }
        Ok(())
    }

    /// Takes the writer lease for one session, failing immediately on contention.
    fn acquire_lease(&self, session: SessionId) -> Result<Lease<'_>, ServiceError> {
        let mut leases = self.leases.lock().map_err(|_| poisoned())?;
        if leases.held.contains_key(&session) {
            return Err(ServiceError::retryable_conflict(
                "session is being mutated by another request",
            ));
        }
        let token = leases.next;
        leases.next = token.wrapping_add(1);
        leases.held.insert(session, token);
        Ok(Lease {
            service: self,
            session,
            token,
        })
    }

    fn require_storage_root(&self) -> Result<&Path, ServiceError> {
        self.storage_root.as_deref().ok_or_else(|| {
            ServiceError::new(
                ErrorCode::NotConfigured,
                "durable storage is not configured",
            )
        })
    }

    fn lock_manager(&self) -> Result<MutexGuard<'_, SessionManager>, ServiceError> {
        self.manager.lock().map_err(|_| poisoned())
    }

    fn lock_registry(&self) -> Result<MutexGuard<'_, Registry>, ServiceError> {
        self.registry.lock().map_err(|_| poisoned())
    }
}

fn request_parts(document: &JsonValue) -> Result<(&str, &JsonValue), ServiceError> {
    let object = document
        .as_object()
        .ok_or_else(|| invalid("request must be a JSON object"))?;
    for key in object.keys() {
        if key != "op" && key != "input" {
            // The client-supplied key is never echoed: its length is client
            // controlled and would make the error envelope unbounded.
            return Err(invalid("unknown field in the request"));
        }
    }
    let op = object
        .get("op")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("`op` must be a string"))?;
    let input = object
        .get("input")
        .ok_or_else(|| invalid("missing required field `input`"))?;
    if input.as_object().is_none() {
        return Err(invalid("`input` must be a JSON object"));
    }
    Ok((op, input))
}

fn lineage_json(lineage: &Lineage) -> Result<JsonValue, ServiceError> {
    Ok(JsonValue::object([
        (
            "parent_session",
            optional_unsigned(lineage.parent_session().map(SessionId::raw))?,
        ),
        (
            "origin_snapshot",
            optional_unsigned(lineage.origin_snapshot().map(SnapshotId::raw))?,
        ),
        ("depth", unsigned(u64::from(lineage.depth()))?),
    ]))
}

fn state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Ready => "ready",
        SessionState::Paused => "paused",
        SessionState::Closed => "closed",
    }
}

fn bytes_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn session_limit() -> ServiceError {
    ServiceError::new(ErrorCode::LimitExceeded, "session limit reached")
}

fn invalid(message: &'static str) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

fn poisoned() -> ServiceError {
    ServiceError::internal("service lock poisoned")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{AppService, ServiceConfig};
    use crate::auth::{Principal, Scope};
    use crate::error::ErrorCode;
    use crate::json::{JsonValue, parse, to_canonical_bytes};
    use crate::limits::ServiceLimits;

    fn service(limits: ServiceLimits) -> AppService {
        AppService::new(ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits,
        })
        .expect("service")
    }

    fn document(op: &str, input: &str) -> Vec<u8> {
        let limits = ServiceLimits::default();
        let value = JsonValue::object([
            ("op", JsonValue::string(op)),
            (
                "input",
                parse(input.as_bytes(), &limits).expect("input object"),
            ),
        ]);
        to_canonical_bytes(&value, &limits).expect("request bytes")
    }

    fn operator() -> Principal {
        Principal::new("operator", [Scope::Read, Scope::Write])
    }

    #[test]
    fn dispatch_returns_a_canonical_envelope() {
        let service = service(ServiceLimits::default());
        let bytes = service
            .dispatch(&operator(), &document("capabilities", "{}"))
            .expect("capabilities");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        assert_eq!(
            envelope.get("op").and_then(JsonValue::as_str),
            Some("capabilities")
        );
        assert!(
            envelope
                .get("result")
                .and_then(JsonValue::as_object)
                .is_some()
        );
        assert_eq!(
            to_canonical_bytes(&envelope, &ServiceLimits::default()).expect("bytes"),
            bytes
        );
    }

    #[test]
    fn unknown_operations_and_malformed_requests_are_typed() {
        let service = service(ServiceLimits::default());
        assert_eq!(
            service
                .dispatch(&operator(), &document("session.nope", "{}"))
                .expect_err("unknown op")
                .code(),
            ErrorCode::UnknownOp
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"{}")
                .expect_err("missing fields")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"{\"op\":1,\"input\":{}}")
                .expect_err("mistyped op")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(
                    &operator(),
                    b"{\"op\":\"capabilities\",\"input\":[],\"x\":1}"
                )
                .expect_err("unknown field")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            service
                .dispatch(&operator(), b"not json")
                .expect_err("parse failure")
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn a_read_only_principal_cannot_mutate() {
        let service = service(ServiceLimits::default());
        let reader = Principal::new("reader", [Scope::Read]);
        assert_eq!(
            service
                .dispatch(&reader, &document("session.create", "{}"))
                .expect_err("forbidden")
                .code(),
            ErrorCode::Forbidden
        );
        assert!(
            service
                .dispatch(&reader, &document("capabilities", "{}"))
                .is_ok(),
            "read scope must allow read operations"
        );
    }

    #[test]
    fn session_count_counts_live_sessions_only() {
        let service = service(ServiceLimits::default());
        assert_eq!(service.session_count().expect("count"), 0);
        let bytes = service
            .dispatch(&operator(), &document("session.create", "{}"))
            .expect("create");
        let envelope = parse(&bytes, &ServiceLimits::default()).expect("envelope");
        assert_eq!(
            envelope
                .get("result")
                .and_then(|result| result.get("session"))
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(service.session_count().expect("count"), 1);
    }
}
