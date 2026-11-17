//! The window-scoped controller lease and the human/agent handoff state machine.
//!
//! [`Controller`] is `Human` (the initial state) or `Agent`. [`Lease`] carries a
//! `generation` and a `sequence`, both incremented on every effective transition; handing
//! off to the controller that already holds the lease is a no-op that reports
//! `changed=false` without touching either counter (`gui.md` §8).
//!
//! Control commands (`step`, `run`, `pulse`) execute only while the active controller is
//! `Human`. Under `Agent` they are rejected with [`ControlError::Denied`], which carries the
//! current generation and sequence and converts into the typed `control_denied`
//! [`GuiError`]. `handoff` itself is always allowed — it is the transition,
//! not a control action — and observation commands always work.
//!
//! The lease is **cooperative, not enforcing**: it prevents a human surface from driving
//! while an agent holds control, and it cannot and does not lock the canonical session. The
//! hard boundaries remain the M6 ones — bearer-token scopes, per-principal ownership and the
//! per-session writer lease. An agent that holds a token can always call `session.run`; the
//! GUI observes the result on its next `session.inspect`. Nothing in this module reaches a
//! channel, a view or a model: it is a three-field value and one comparison.

use std::fmt;

use crate::GuiError;

/// One controller of a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Controller {
    /// The human at the terminal, the initial and default controller.
    Human,
    /// The agent that took the lease with `handoff agent`.
    Agent,
}

impl Controller {
    /// Returns the canonical command and block name of this controller.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// Parses a `handoff` argument.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] for any name other than `human` and `agent`.
    pub fn parse(name: &str) -> Result<Self, GuiError> {
        match name {
            "human" => Ok(Self::Human),
            "agent" => Ok(Self::Agent),
            other => Err(GuiError::bad_value(
                "controller",
                &format!("`{other}` is neither human nor agent"),
            )),
        }
    }
}

impl fmt::Display for Controller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The typed refusal of a control command.
///
/// It is a distinct type rather than a bare [`GuiError`] so the decision ("this command
/// needs the human lease") is a value a caller can inspect before converting it; the
/// conversion is the only place that maps it onto the documented `control_denied` code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// The agent holds the lease, so a control command is refused.
    Denied {
        /// The lease generation at the moment of the rejection.
        generation: u64,
        /// The lease sequence at the moment of the rejection.
        sequence: u64,
    },
}

impl ControlError {
    /// Returns the lease generation the rejection reports.
    #[must_use]
    pub const fn generation(self) -> u64 {
        match self {
            Self::Denied { generation, .. } => generation,
        }
    }

    /// Returns the lease sequence the rejection reports.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        match self {
            Self::Denied { sequence, .. } => sequence,
        }
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied {
                generation,
                sequence,
            } => write!(
                formatter,
                "the agent holds control (generation {generation}, sequence {sequence})"
            ),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<ControlError> for GuiError {
    fn from(error: ControlError) -> Self {
        Self::control_denied(error.generation(), error.sequence())
    }
}

/// The typed result of one `handoff` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Handoff {
    controller: Controller,
    previous: Controller,
    changed: bool,
    generation: u64,
    sequence: u64,
}

impl Handoff {
    /// Returns the controller that now holds the lease.
    #[must_use]
    pub const fn controller(self) -> Controller {
        self.controller
    }

    /// Returns the controller that held the lease before the command.
    #[must_use]
    pub const fn previous(self) -> Controller {
        self.previous
    }

    /// Returns whether the handoff was effective; `false` means it was a no-op.
    #[must_use]
    pub const fn changed(self) -> bool {
        self.changed
    }

    /// Returns the generation after the command.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Returns the sequence after the command.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// The window-scoped controller lease: one controller, one generation, one sequence.
///
/// The lease is a plain value (`Copy`), so a renderer can be handed a consistent snapshot
/// of it and two frames of the same window cannot disagree about who is in control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lease {
    controller: Controller,
    generation: u64,
    sequence: u64,
}

impl Lease {
    /// Creates the initial lease: `human`, generation 0, sequence 0.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            controller: Controller::Human,
            generation: 0,
            sequence: 0,
        }
    }

    /// Returns the active controller.
    #[must_use]
    pub const fn controller(&self) -> Controller {
        self.controller
    }

    /// Returns the lease generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the lease sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Transfers the lease to `requested` and returns the typed transition report.
    ///
    /// An effective transition increments both counters, saturating rather than wrapping;
    /// asking the current controller to take the lease is a no-op that reports
    /// `changed=false` and leaves both counters untouched.
    pub fn handoff(&mut self, requested: Controller) -> Handoff {
        let previous = self.controller;
        if requested == previous {
            return Handoff {
                controller: previous,
                previous,
                changed: false,
                generation: self.generation,
                sequence: self.sequence,
            };
        }
        self.controller = requested;
        self.generation = self.generation.saturating_add(1);
        self.sequence = self.sequence.saturating_add(1);
        Handoff {
            controller: requested,
            previous,
            changed: true,
            generation: self.generation,
            sequence: self.sequence,
        }
    }

    /// Authorizes one control command, which requires the human lease.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::Denied`] carrying the current generation and sequence while
    /// the agent holds the lease.
    pub fn authorize_control(&self) -> Result<(), ControlError> {
        match self.controller {
            Controller::Human => Ok(()),
            Controller::Agent => Err(ControlError::Denied {
                generation: self.generation,
                sequence: self.sequence,
            }),
        }
    }
}

impl Default for Lease {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlError, Controller, Lease};
    use crate::{GuiError, GuiErrorCode};

    #[test]
    fn a_new_lease_is_human_with_zeroed_counters() {
        let lease = Lease::new();
        assert_eq!(lease.controller(), Controller::Human);
        assert_eq!(lease.generation(), 0);
        assert_eq!(lease.sequence(), 0);
        assert_eq!(Lease::default(), lease);
        assert_eq!(lease.authorize_control(), Ok(()));
    }

    #[test]
    fn an_effective_handoff_increments_both_counters() {
        let mut lease = Lease::new();
        let report = lease.handoff(Controller::Agent);
        assert!(report.changed());
        assert_eq!(report.controller(), Controller::Agent);
        assert_eq!(report.previous(), Controller::Human);
        assert_eq!(report.generation(), 1);
        assert_eq!(report.sequence(), 1);
        assert_eq!(lease.generation(), 1);
        assert_eq!(lease.sequence(), 1);
        assert_eq!(lease.controller(), Controller::Agent);

        let report = lease.handoff(Controller::Human);
        assert!(report.changed());
        assert_eq!(report.previous(), Controller::Agent);
        assert_eq!(report.generation(), 2);
        assert_eq!(report.sequence(), 2);
        assert_eq!(lease.controller(), Controller::Human);
    }

    #[test]
    fn handing_off_to_the_current_controller_is_a_no_op() {
        let mut lease = Lease::new();
        let first = lease.handoff(Controller::Human);
        assert!(!first.changed());
        assert_eq!(first.previous(), Controller::Human);
        assert_eq!(first.generation(), 0);
        assert_eq!(first.sequence(), 0);

        lease.handoff(Controller::Agent);
        let repeated = lease.handoff(Controller::Agent);
        assert!(!repeated.changed());
        assert_eq!(repeated.generation(), 1, "the counters do not move");
        assert_eq!(repeated.sequence(), 1);
        assert_eq!(lease.generation(), 1);
        assert_eq!(lease.sequence(), 1);
    }

    #[test]
    fn control_is_authorized_only_under_human() {
        let mut lease = Lease::new();
        lease.handoff(Controller::Agent);
        let denied = lease.authorize_control().expect_err("agent holds control");
        assert_eq!(
            denied,
            ControlError::Denied {
                generation: 1,
                sequence: 1,
            }
        );
        assert_eq!(denied.generation(), 1);
        assert_eq!(denied.sequence(), 1);
        assert_eq!(
            denied.to_string(),
            "the agent holds control (generation 1, sequence 1)"
        );

        let error: GuiError = denied.into();
        assert_eq!(error.code(), GuiErrorCode::ControlDenied);
        assert!(matches!(
            error,
            GuiError::ControlDenied {
                generation: 1,
                sequence: 1
            }
        ));
        assert!(error.to_string().starts_with("control_denied: "));

        lease.handoff(Controller::Human);
        assert_eq!(lease.authorize_control(), Ok(()));
    }

    #[test]
    fn controllers_parse_and_display_their_documented_names() {
        assert_eq!(
            Controller::parse("human").expect("human"),
            Controller::Human
        );
        assert_eq!(
            Controller::parse("agent").expect("agent"),
            Controller::Agent
        );
        for name in ["", "Human", "Agent", "human ", "daemon"] {
            let error = Controller::parse(name).expect_err("unknown controller");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("controller"));
        }
        assert_eq!(Controller::Human.as_str(), "human");
        assert_eq!(Controller::Agent.as_str(), "agent");
        assert_eq!(Controller::Human.to_string(), "human");
        assert_eq!(Controller::Agent.to_string(), "agent");
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let mut lease = Lease {
            controller: Controller::Human,
            generation: u64::MAX,
            sequence: u64::MAX,
        };
        let report = lease.handoff(Controller::Agent);
        assert_eq!(report.generation(), u64::MAX);
        assert_eq!(report.sequence(), u64::MAX);
        assert!(report.changed());
    }
}
