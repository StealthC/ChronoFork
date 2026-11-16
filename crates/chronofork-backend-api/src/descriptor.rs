//! Backend identity and capability reporting.
//!
//! Capabilities describe guarantees the backend actually implements. A method
//! existing on the trait is not a capability: the host must consult this report
//! before exposing a feature to a user, script or agent.

use std::fmt;

/// Stable identifier for a backend implementation family.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BackendId(String);

impl BackendId {
    /// Creates a backend identifier.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrows the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Feature guarantees reported by one backend.
///
/// A `false` value is a hard statement that the feature is not available
/// through this backend, not merely that the host has not implemented a UI for
/// it. Host-provided features must be reported separately by the host layer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    /// The backend can advance emulated time.
    pub execution: bool,
    /// The backend can produce and consume full machine snapshots.
    pub save_states: bool,
    /// Debugger reads of emulated memory are supported.
    pub memory_read: bool,
    /// Debugger writes of emulated memory are supported.
    pub memory_write: bool,
    /// CPU registers can be inspected.
    pub register_inspection: bool,
    /// Execution can advance by a single backend-defined quantum.
    pub instruction_stepping: bool,
    /// Conditional code breakpoints can be installed (not implemented by a
    /// backend; host debuggers implement them over `inspect`).
    pub breakpoints: bool,
    /// Memory watchpoints can be installed and reported by the backend bus.
    pub watchpoints: bool,
    /// Bounded typed bus, device and interrupt events are observable through
    /// [`crate::DebugAccess`].
    pub debug_events: bool,
    /// Structured execution tracing is available (not yet implemented).
    pub tracing: bool,
    /// A snapshot can be restored into a fresh independent instance.
    pub state_cloning: bool,
    /// Identical inputs from identical states are expected to converge.
    pub deterministic_replay: bool,
    /// The backend produces video frames.
    pub rendering: bool,
    /// The backend produces audio samples.
    pub audio: bool,
}

impl Capabilities {
    /// No optional capability is advertised.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            execution: false,
            save_states: false,
            memory_read: false,
            memory_write: false,
            register_inspection: false,
            instruction_stepping: false,
            breakpoints: false,
            watchpoints: false,
            debug_events: false,
            tracing: false,
            state_cloning: false,
            deterministic_replay: false,
            rendering: false,
            audio: false,
        }
    }
}

/// Descriptive metadata for a backend family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendDescriptor {
    id: BackendId,
    display_name: String,
    version: String,
    capabilities: Capabilities,
    test_backend: bool,
}

impl BackendDescriptor {
    /// Creates a descriptor.
    #[must_use]
    pub fn new(
        id: BackendId,
        display_name: impl Into<String>,
        version: impl Into<String>,
        capabilities: Capabilities,
        test_backend: bool,
    ) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            version: version.into(),
            capabilities,
            test_backend,
        }
    }

    /// Returns the stable backend identifier.
    #[must_use]
    pub fn id(&self) -> &BackendId {
        &self.id
    }

    /// Returns a human-readable name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the backend implementation version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the declared capabilities.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Returns whether this is a synthetic test implementation.
    #[must_use]
    pub const fn is_test_backend(&self) -> bool {
        self.test_backend
    }
}
