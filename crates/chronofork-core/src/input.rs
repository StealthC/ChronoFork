//! Backend-defined input for one execution step.
//!
//! The bit assignment is a backend contract, not a ChronoFork-wide controller
//! layout. A future versioned input model will add ports and analog axes; the
//! current shape stays deliberately small so the first backend interface can
//! be implemented and tested without pretending to support hardware it does
//! not have.

/// Input supplied to one backend step.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputFrame {
    buttons: u32,
}

impl InputFrame {
    /// No buttons pressed.
    pub const NEUTRAL: Self = Self { buttons: 0 };

    /// Creates an input frame from a backend-defined bit mask.
    #[must_use]
    pub const fn new(buttons: u32) -> Self {
        Self { buttons }
    }

    /// Returns the backend-defined bit mask.
    #[must_use]
    pub const fn buttons(self) -> u32 {
        self.buttons
    }
}

#[cfg(test)]
mod tests {
    use super::InputFrame;

    #[test]
    fn neutral_input_has_no_buttons() {
        assert_eq!(InputFrame::NEUTRAL.buttons(), 0);
    }

    #[test]
    fn input_round_trips_the_mask() {
        let input = InputFrame::new(0x00ff_00ff);
        assert_eq!(input.buttons(), 0x00ff_00ff);
    }
}
