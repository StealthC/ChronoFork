//! Recorded deterministic batch prefix: the number of steps `k` and their inputs.
//!
//! A batch run is reconstructible from its reported `performed` count and the input
//! frames it consumed, because the loop is deterministic and never reads host time.
//! This record is deliberately independent of the movie formats (ADR-0004/0006): movie
//! policy is W2.8 and nothing here changes v1/v2 behavior.

use chronofork_backend_api::{
    BatchOutcome, InputSequence, RunControl, StopSet, VerificationPolicy,
};
use chronofork_core::SessionId;

use crate::{SessionError, SessionManager};

/// The deterministic prefix of one batch run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    performed: u64,
    inputs: InputSequence,
}

impl RunRecord {
    /// Records `performed` steps and the explicit input frames they consumed.
    ///
    /// Only `min(performed, inputs.len())` explicit frames are stored; neutral padding is
    /// implicit, so a long neutral run records no per-step frames and never trips the
    /// input-sequence cap. The full performed count is retained for replay.
    #[must_use]
    pub fn new(performed: u64, inputs: &InputSequence) -> Self {
        Self {
            performed,
            inputs: inputs.prefix(performed),
        }
    }

    /// Records the prefix reported by a finished batch outcome.
    #[must_use]
    pub fn from_outcome(outcome: &BatchOutcome, inputs: &InputSequence) -> Self {
        Self::new(outcome.performed, inputs)
    }

    /// Returns the number of steps in the prefix.
    #[must_use]
    pub const fn performed(&self) -> u64 {
        self.performed
    }

    /// Returns the recorded inputs (neutral padding is implicit past the end).
    #[must_use]
    pub fn inputs(&self) -> &InputSequence {
        &self.inputs
    }

    /// Replays the recorded prefix under the caller's stop set and policy.
    ///
    /// The result must equal the recorded batch prefix; the test suite asserts that.
    ///
    /// # Errors
    ///
    /// Returns the session/backend rejection of [`SessionManager::run_until`].
    pub fn replay(
        &self,
        manager: &mut SessionManager,
        session: SessionId,
        stop: &StopSet,
        policy: VerificationPolicy,
        control: &RunControl,
    ) -> Result<BatchOutcome, SessionError> {
        manager.run_until(session, stop, self.performed, &self.inputs, policy, control)
    }
}
