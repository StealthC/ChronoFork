//! Resource limits shared by every service operation and transport adapter.

/// Bounds applied to service inputs and outputs.
///
/// One value is used by the canonical JSON codec, the operation dispatcher and the
/// HTTP adapter so a limit is declared once. Raising a default is a documented
/// contract change, not an ordinary configuration tweak.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceLimits {
    /// Maximum size in bytes of one serialized request.
    pub max_request_bytes: usize,
    /// Maximum size in bytes of one canonical response.
    pub max_response_bytes: usize,
    /// Maximum number of steps one `session.run` request may advance.
    pub max_steps_per_request: u64,
    /// Maximum number of bytes one `session.read_memory` request may read.
    pub max_memory_read: usize,
    /// Maximum number of registered sessions the service will create.
    ///
    /// Closed sessions stay registered in the session layer (there is no removal API),
    /// so they continue to count toward this bound for the process lifetime; only
    /// `AppService::session_count()` reports live sessions.
    pub max_sessions: usize,
    /// Maximum number of durable sessions or snapshots one list response may carry.
    pub max_store_entries: usize,
    /// Maximum number of experiment checkpoints.
    pub max_checkpoints: usize,
    /// Maximum JSON nesting depth accepted or produced.
    pub json_max_depth: usize,
    /// Host wall-clock budget of one `session.run`, in milliseconds.
    ///
    /// This is a host policy, never emulated time (ADR-0016 §2, K3): the deadline
    /// chooses how many steps `k` are applied, and the replay records `k` plus the
    /// inputs, so canonical evolution is unaffected. A caller may lower it per
    /// request with `max_wall_ms`; it can never raise it above this cap.
    pub max_run_wall_ms: u64,
    /// Maximum number of retained entries in the operation journal.
    ///
    /// Admission is checked **before** an operation starts and a retained entry is
    /// never evicted (ADR-0016 §2, K4/F5); when the journal is full the new request
    /// is rejected with a typed `limit` instead of dropping a retained result. This
    /// bound covers the **outcomes**; the **identity** memory (the keys the service has
    /// ever used, which is what makes a retry safe) is bounded separately by
    /// `max(max_operation_journal, 1024)` and is likewise never silently evicted.
    pub max_operation_journal: usize,
    /// Seconds a **finished** journal entry stays reconcilable, counted from
    /// completion.
    ///
    /// An active operation never expires; beyond this horizon the request key is not
    /// recognized and the outcome is `unknown_outcome` (ADR-0016 §2, V3-3).
    pub operation_ttl: u64,
    /// Maximum number of closed session entries retained before forced cleanup.
    ///
    /// Closed entries are bounded independently of the live count so a low live count
    /// cannot hide unbounded closed state (ADR-0016 §3, K9/V3-2). Cleanup itself is
    /// W3.3; this is the declared bound it enforces.
    pub max_closed_retained: usize,
    /// Seconds a closed session entry may be retained before forced cleanup.
    pub closed_retention_ttl: u64,
    /// Maximum number of checkpoints one session may store.
    ///
    /// Checked before the checkpoint is allocated (ADR-0016 §3): a rejected
    /// `session.snapshot` leaves the session and its checkpoints untouched.
    pub max_snapshots_per_session: usize,
    /// Maximum serialized bytes of checkpoints one session may retain.
    ///
    /// The accounting unit is the serialized checkpoint record, which is exactly the
    /// `bytes_length` a `session.snapshot` reports, so the bound is observable
    /// without inspecting private state.
    pub max_bytes_per_session: usize,
    /// Best-effort global ceiling on the checkpoint bytes every registered session
    /// retains.
    ///
    /// ADR-0016 §3 declares the global store-byte quota **best-effort**: the store
    /// writer lock is per key, so a cross-key check cannot be atomic. This build
    /// enforces it at admission with one synchronized counter and declares that a
    /// session closed after it was admitted may leave its bytes counted until it is
    /// reaped. The acceptance gate does not claim a strict global quota.
    pub max_store_bytes: usize,
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: 1024 * 1024,
            max_response_bytes: 1024 * 1024,
            max_steps_per_request: 100_000,
            max_memory_read: 4096,
            max_sessions: 1024,
            max_store_entries: 256,
            max_checkpoints: 32,
            json_max_depth: 32,
            max_run_wall_ms: 30_000,
            max_operation_journal: 1_024,
            operation_ttl: 600,
            max_closed_retained: 64,
            closed_retention_ttl: 300,
            max_snapshots_per_session: 64,
            max_bytes_per_session: 64 * 1024 * 1024,
            max_store_bytes: 1024 * 1024 * 1024,
        }
    }
}
