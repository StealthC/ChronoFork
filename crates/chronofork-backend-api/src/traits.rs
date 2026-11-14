//! Backend and factory traits.

use chronofork_core::{InputFrame, StateHash, Tick};

use crate::descriptor::{BackendDescriptor, Capabilities};
use crate::error::BackendError;
use crate::inspection::MachineInspection;
use crate::snapshot::BackendSnapshot;

/// Result of one execution step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    /// Tick after the step.
    pub tick: Tick,
    /// Digest of state after the step.
    pub state_hash: StateHash,
    /// Whether the machine halted during this step.
    pub halted: bool,
}

/// A single independent emulated machine.
///
/// Implementations must not share mutable state with another `Backend`
/// instance. Read-only configuration (for example an owned, immutable program)
/// may be shared through `Arc` when safe.
pub trait Backend: Send {
    /// Returns immutable descriptive metadata.
    fn descriptor(&self) -> &BackendDescriptor;

    /// Advances the machine by one backend-defined quantum.
    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError>;

    /// Captures complete mutable state.
    fn snapshot(&self) -> Result<BackendSnapshot, BackendError>;

    /// Replaces complete mutable state with a snapshot produced by a compatible
    /// backend instance.
    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError>;

    /// Returns a side-effect-free debugger view.
    fn inspect(&self) -> Result<MachineInspection, BackendError>;

    /// Reads emulated memory without side effects.
    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError>;

    /// Writes emulated memory through a debugger-oriented path.
    ///
    /// This is not an emulated-bus write and must not trigger side effects that
    /// would not occur at the current execution point.
    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError>;
}

/// Creates fresh, independent backend instances.
///
/// A factory is shared by a session manager. It must not return two handles to
/// the same mutable machine.
pub trait BackendFactory: Send + Sync {
    /// Returns immutable descriptive metadata for instances produced here.
    fn descriptor(&self) -> &BackendDescriptor;

    /// Creates a fresh machine in its initial deterministic state.
    fn create(&self) -> Result<Box<dyn Backend>, BackendError>;

    /// Reports capabilities without constructing an instance.
    fn capabilities(&self) -> Capabilities {
        self.descriptor().capabilities()
    }
}
