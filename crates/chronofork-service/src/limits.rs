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
        }
    }
}
