//! Backend-defined input for one execution step.
//!
//! The bit, port, axis and key assignments are a backend contract, not a
//! ChronoFork-wide controller layout. The schema-2 shape adds fixed device
//! fields on top of the v1 button mask so TAS records can carry multi-device
//! input without changing the `Backend::step` signature.
//!
//! A backend documents which devices it consumes. Fields it does not consume
//! have no emulated effect; a frame with neutral extra fields is exactly the
//! v1 frame and keeps the v1 state trajectory.

/// Input supplied to one backend step.
///
/// Schema 2 carries three device classes:
///
/// - `ports`: four 32-bit digital port masks. Port 0 is the v1 backend-defined
///   button mask; ports 1 to 3 are additional digital ports.
/// - `axes`: four signed 16-bit analog axes, with `0` meaning neutral.
/// - `keys`: a 64-bit keyboard bitmap, where bit `i` represents key `i`.
///
/// The bit/port ownership inside each field is backend-defined. A neutral
/// frame has every field at zero and must not change backend state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputFrame {
    ports: [u32; 4],
    axes: [i16; 4],
    keys: u64,
}

impl InputFrame {
    /// No port bits, all axes at neutral zero and no keys pressed.
    pub const NEUTRAL: Self = Self {
        ports: [0; 4],
        axes: [0; 4],
        keys: 0,
    };

    /// Creates a v1-compatible frame from a backend-defined bit mask.
    ///
    /// The mask becomes port 0; ports 1 to 3, the axes and the keyboard stay
    /// neutral, so the frame is semantically the v1 frame.
    #[must_use]
    pub const fn new(buttons: u32) -> Self {
        Self {
            ports: [buttons, 0, 0, 0],
            axes: [0; 4],
            keys: 0,
        }
    }

    /// Creates a schema-2 multi-device frame from its device fields.
    #[must_use]
    pub const fn with_devices(ports: [u32; 4], axes: [i16; 4], keys: u64) -> Self {
        Self { ports, axes, keys }
    }

    /// Returns port 0, the v1 backend-defined button mask.
    #[must_use]
    pub const fn buttons(self) -> u32 {
        self.ports[0]
    }

    /// Returns all four digital port masks; index 0 is the v1 button mask.
    #[must_use]
    pub const fn ports(self) -> [u32; 4] {
        self.ports
    }

    /// Returns the four signed analog axes; `0` is neutral.
    #[must_use]
    pub const fn axes(self) -> [i16; 4] {
        self.axes
    }

    /// Returns the keyboard bitmap; bit `i` represents key `i`.
    #[must_use]
    pub const fn keys(self) -> u64 {
        self.keys
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

    #[test]
    fn neutral_frames_are_all_zero_and_equal() {
        let neutral = InputFrame::NEUTRAL;
        assert_eq!(neutral, InputFrame::default());
        assert_eq!(neutral, InputFrame::new(0));
        assert_eq!(neutral, InputFrame::with_devices([0; 4], [0; 4], 0));
        assert_eq!(neutral.ports(), [0; 4]);
        assert_eq!(neutral.axes(), [0; 4]);
        assert_eq!(neutral.keys(), 0);
    }

    #[test]
    fn v1_constructor_sets_only_port_zero() {
        let input = InputFrame::new(0x8000_0001);
        assert_eq!(input.buttons(), 0x8000_0001);
        assert_eq!(input.ports(), [0x8000_0001, 0, 0, 0]);
        assert_eq!(input.axes(), [0; 4]);
        assert_eq!(input.keys(), 0);
        assert_eq!(
            input,
            InputFrame::with_devices([0x8000_0001, 0, 0, 0], [0; 4], 0)
        );
    }

    #[test]
    fn multi_device_fields_round_trip_including_their_extremes() {
        let ports = [0, u32::MAX, 0xaaaa_5555, 1];
        let axes = [i16::MIN, -1, 1, i16::MAX];
        let keys = u64::MAX;
        let input = InputFrame::with_devices(ports, axes, keys);
        assert_eq!(input.ports(), ports);
        assert_eq!(input.buttons(), 0);
        assert_eq!(input.axes(), axes);
        assert_eq!(input.keys(), keys);
        assert_eq!(input, InputFrame::with_devices(ports, axes, keys));
        assert_ne!(input, InputFrame::NEUTRAL);
    }

    #[test]
    fn keyboard_bits_keep_their_defined_positions() {
        let input = InputFrame::with_devices([0; 4], [0; 4], (1 << 63) | 1);
        assert_eq!(input.keys() & 1, 1);
        assert_eq!(input.keys() >> 63, 1);
        assert_eq!(input.keys().count_ones(), 2);
    }

    #[test]
    fn equality_and_hash_include_every_device_field() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn digest(input: InputFrame) -> u64 {
            let mut hasher = DefaultHasher::new();
            input.hash(&mut hasher);
            hasher.finish()
        }

        let base = InputFrame::with_devices([1, 2, 3, 4], [5, 6, 7, 8], 9);
        assert_eq!(digest(base), digest(base));
        let changed_port = InputFrame::with_devices([1, 2, 3, 0], [5, 6, 7, 8], 9);
        let changed_axis = InputFrame::with_devices([1, 2, 3, 4], [5, 6, 7, 0], 9);
        let changed_keys = InputFrame::with_devices([1, 2, 3, 4], [5, 6, 7, 8], 8);
        assert_ne!(base, changed_port);
        assert_ne!(base, changed_axis);
        assert_ne!(base, changed_keys);
        assert_ne!(digest(base), digest(changed_port));
        assert_ne!(digest(base), digest(changed_axis));
        assert_ne!(digest(base), digest(changed_keys));
    }
}
