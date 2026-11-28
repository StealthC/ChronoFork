//! Backend and factory traits.

use chronofork_core::{InputFrame, StateHash, Tick};

use crate::batch::{
    BatchOutcome, BatchSignals, InputSequence, RunControl, StopBudget, StopSet, VerificationPolicy,
};
use crate::debug::DebugAccess;
use crate::descriptor::{BackendDescriptor, Capabilities};
use crate::error::BackendError;
use crate::inspection::{MachineInspection, MachineStatus, MemoryWindow};
use crate::media::MediaSource;
use crate::snapshot::BackendSnapshot;

/// Result of one **ordinary** execution step.
///
/// This is the digest-free target shape from the W2 design (D1/W2-3): the type
/// carries no state digest, so an ordinary step cannot be mistaken for a verified
/// one. [`Backend::step`] always returns `verified = false`; a caller that needs a
/// per-step digest uses [`Backend::step_verified`] (which returns
/// [`VerifiedStepOutcome`]) or calls [`Backend::state_hash`] explicitly.
///
/// ```compile_fail
/// # use chronofork_backend_api::StepOutcome;
/// fn digest(outcome: StepOutcome) -> chronofork_core::StateHash {
///     // `state_hash` was removed from the target shape (W2-3).
///     outcome.state_hash
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    /// Tick after the step.
    pub tick: Tick,
    /// Whether the machine halted during this step.
    pub halted: bool,
    /// Whether this outcome was produced by a verified path.
    ///
    /// Always `false` for [`Backend::step`]; the flag is retained by the design's
    /// target shape so a generic replay harness can tell the two producers apart.
    pub verified: bool,
}

/// Result of one verified execution step.
///
/// A distinct type from [`StepOutcome`], so a harness that needs a per-step
/// digest cannot mistake an ordinary outcome for a verified one. The type itself
/// is the verification guarantee: [`Backend::step_verified`] is its only producer,
/// so there is no always-`true` flag to check or to get wrong.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedStepOutcome {
    /// Tick after the step.
    pub tick: Tick,
    /// Whether the machine halted during this step.
    pub halted: bool,
    /// Digest of the state after the step.
    pub state_hash: StateHash,
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
        })
    }

    /// Advances a bounded batch with the caller's inputs, budget, stop set and policy.
    ///
    /// This is the only batch primitive. The default is the reference implementation in
    /// [`crate::batch`]; it steps, checks the budget, the pre-step conditions, the halt
    /// signal and the post-step conditions under the documented precedence, and reads
    /// host time only for the caller's absolute deadline at a step boundary. A backend
    /// override may only avoid the per-step trait call; it must keep the same grammar,
    /// precedence and result shape.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Limit`] for an input sequence longer than a `Steps` budget
    /// **before** any step runs, and propagates step/digest errors otherwise.
    fn run_batch(
        &mut self,
        inputs: &InputSequence,
        budget: StopBudget,
        stop: &StopSet,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, BackendError> {
        crate::batch::run_batch_default(self, inputs, budget, stop, policy, control)
    }

    /// Returns cheap observations for the batch loop without computing a digest.
    ///
    /// The default exposes only the halt flag. A backend that can answer the instruction
    /// pointer or frame index cheaply overrides this so `Pc`/`Frame` stop conditions work
    /// without an inspection (which may hash).
    #[must_use]
    fn batch_signals(&self) -> BatchSignals {
        BatchSignals::default()
    }

    /// Captures complete mutable state.
    fn snapshot(&self) -> Result<BackendSnapshot, BackendError>;

    /// Replaces complete mutable state with a snapshot produced by a compatible
    /// backend instance.
    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError>;

    /// Returns a cheap status view: no digest and no memory copy.
    ///
    /// The register maps must respect [`crate::MAX_STATUS_REGISTERS`] together and keep the
    /// CPU/core file separate from device pseudo-registers. A backend implements this
    /// from its own fields; it must not serialize state or copy RAM.
    ///
    /// The provided default projects [`Backend::inspect`], which computes no digest and
    /// copies no memory; production backends override it with a direct field read.
    fn status(&self) -> Result<MachineStatus, BackendError> {
        let inspection = self.inspect()?;
        Ok(MachineStatus {
            tick: inspection.tick,
            halted: inspection.halted,
            cpu_registers: inspection.cpu_registers,
            device_registers: inspection.device_registers,
            frame_index: inspection.frame_index,
        })
    }

    /// Returns the on-demand detailed view: registers only, no digest, no memory copy.
    ///
    /// A caller that needs a digest calls [`Backend::state_hash`] explicitly. Memory is
    /// obtained through [`Backend::inspect_with_memory`] or `read_memory`.
    fn inspect(&self) -> Result<MachineInspection, BackendError>;

    /// Returns the backend's default inspection memory window `(base, length)`.
    ///
    /// Used by consumers that preserve the detailed view's memory metadata without
    /// copying it unless they ask for the window. The default is an empty window.
    #[must_use]
    fn inspection_memory_window(&self) -> (u64, usize) {
        (0, 0)
    }

    /// Returns the detailed view with an opt-in memory window.
    ///
    /// The default composes `inspect` and `read_memory`, so a backend that implements
    /// both gets the window without a second copy path.
    ///
    /// # Errors
    ///
    /// Propagates the inspection or memory-read error.
    fn inspect_with_memory(
        &self,
        base: u64,
        length: usize,
    ) -> Result<MachineInspection, BackendError> {
        let inspection = self.inspect()?;
        let bytes = self.read_memory(base, length)?;
        Ok(inspection.with_memory(MemoryWindow { base, bytes }))
    }

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
