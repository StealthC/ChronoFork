//! Backend and factory traits.

use chronofork_core::{InputFrame, StateHash, Tick};

use crate::debug::DebugAccess;
use crate::descriptor::{BackendDescriptor, Capabilities};
use crate::error::BackendError;
use crate::inspection::MachineInspection;
use crate::media::MediaSource;
use crate::snapshot::BackendSnapshot;

/// Result of one execution step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    /// Tick after the step.
    pub tick: Tick,
    /// Digest of state after the step, when the path produced one.
    ///
    /// [`Backend::step`] returns `None` from W2-2 onward (it computed `Some` only
    /// during the W2-1 migration landing); [`Backend::step_verified`] always
    /// returns a digest as [`VerifiedStepOutcome::state_hash`]. The field is
    /// removed entirely at W2-3, so consumers that need a digest must use the
    /// verified path now rather than treat this field as a guarantee.
    pub state_hash: Option<StateHash>,
    /// Whether the machine halted during this step.
    pub halted: bool,
    /// Whether this outcome was produced by a verified path.
    ///
    /// `false` for [`Backend::step`] (the ordinary path) and `true` for
    /// [`Backend::step_verified`]; these are the only two producers of the flag.
    pub verified: bool,
}

/// Result of one verified execution step.
///
/// A distinct type from [`StepOutcome`], so a harness that needs a per-step
/// digest cannot mistake an ordinary outcome for a verified one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedStepOutcome {
    /// Tick after the step.
    pub tick: Tick,
    /// Whether the machine halted during this step.
    pub halted: bool,
    /// Digest of the state after the step.
    pub state_hash: StateHash,
    /// Always `true`; [`Backend::step_verified`] is the only producer.
    pub verified: bool,
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

    /// Computes the canonical state digest without changing any state.
    ///
    /// There is exactly one canonical computation in the implementation, and the
    /// verified path calls it. A backend that hashes a coarse view for speed must
    /// still keep the serialized field layout in one writer; this method must never
    /// become a second serializer.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the implementation cannot produce the digest.
    fn state_hash(&self) -> Result<StateHash, BackendError>;

    /// Advances the machine by one quantum and produces a per-step digest.
    ///
    /// The default is [`Backend::step`] followed by exactly one
    /// [`Backend::state_hash`] on the resulting state, so the verified digest can
    /// never drift from the canonical source. A backend override may reorder that
    /// composition (for example to hash inside one batch loop) but must never
    /// recompute the serializer.
    ///
    /// # Errors
    ///
    /// Returns the backend's typed step or digest error.
    fn step_verified(&mut self, input: InputFrame) -> Result<VerifiedStepOutcome, BackendError> {
        let outcome = self.step(input)?;
        let state_hash = self.state_hash()?;
        Ok(VerifiedStepOutcome {
            tick: outcome.tick,
            halted: outcome.halted,
            state_hash,
            verified: true,
        })
    }

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

    /// Returns the optional bounded debug observation surface.
    ///
    /// The default is `None`: a backend that cannot observe its own bus keeps
    /// the default and must not advertise
    /// [`Capabilities::watchpoints`] or [`Capabilities::debug_events`].
    ///
    /// Observation is debug data, not canonical state: it must never enter a
    /// snapshot, the state hash, the configuration hash or the payload version,
    /// and restoring a snapshot must not rewind it. Debugger `read_memory` and
    /// `write_memory` calls must not produce events either. See
    /// [ADR-0010](../../../docs/adr/0010-debugging-depth.md).
    fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> {
        None
    }

    /// Returns the optional media capture surface.
    ///
    /// The default is `None`: a backend that produces no video frames or audio
    /// chunks keeps the default and must not advertise
    /// [`Capabilities::rendering`] or [`Capabilities::audio`].
    ///
    /// Media is output derived from canonical state, not canonical state
    /// itself: it must never enter a snapshot, the state hash, the
    /// configuration hash or the payload version, and restoring a snapshot
    /// must not rewind a queued frame. A frame is produced deterministically
    /// from emulated state and the master-cycle timeline, never from host time.
    fn media_mut(&mut self) -> Option<&mut dyn MediaSource> {
        None
    }

    /// Declares an upper bound, in bytes, for the payload of any `snapshot()`.
    ///
    /// A session layer enforces a byte quota **before** it captures a checkpoint, and it
    /// can only do that when the payload size is bounded: when the state has not changed
    /// since the last capture the previous observation is exact, but a state that a
    /// `step`, a trace or a debug write changed has no known size. A backend whose
    /// payload size can change while it executes **must** declare a bound here that its
    /// `snapshot()` guarantees; the layer then refuses a capture that cannot fit the
    /// quota without allocating it.
    ///
    /// The default is `None`: the backend does not commit to a bound, and a
    /// quota-checked capture of a changed state is then refused (`limit_exceeded`)
    /// instead of attempted. A backend with a fixed-size payload (the common case)
    /// simply returns that size.
    fn snapshot_payload_bound(&self) -> Option<usize> {
        None
    }
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
