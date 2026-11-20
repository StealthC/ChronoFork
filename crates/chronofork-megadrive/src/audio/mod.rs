//! Mega Drive audio devices.
//!
//! Phase 8.4 adds the YM2612 six-channel FM synthesiser ([`ym2612`]), the
//! SN76489-compatible PSG ([`psg`]) and the integer [`mixer`]/[`resample`] path
//! that turns their outputs into 48 kHz stereo frames. Nothing here talks to a
//! host audio device: the devices are deterministic integer models that the
//! backend advances by elapsed master cycles, and their sample functions are
//! pure functions of generated state.

pub mod mixer;
pub mod psg;
pub mod resample;
pub mod ym2612;

use std::error::Error;
use std::fmt;

use chronofork_core::codec::CodecError;

pub use mixer::{AudioMixer, OUTPUT_CHANNELS, OUTPUT_SAMPLE_RATE_HZ};
pub use psg::Psg;
pub use resample::{RationalResampler, ResampleError};
pub use ym2612::{EgState, EnvelopeView, Ym2612, YmBank, YmIrq, YmPort};

/// A malformed audio-device snapshot.
///
/// The YM2612, PSG and mixer serializers share this typed error so the backend
/// can report every rejected device payload as one invalid-snapshot cause while
/// keeping the failing constraint specific.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioSnapshotError {
    /// The payload ended before the fixed snapshot length.
    Truncated,
    /// The payload carried bytes past the fixed snapshot length.
    TrailingBytes,
    /// The snapshot magic did not match the device format.
    Magic,
    /// The device snapshot version is not supported.
    Version {
        /// Version byte found in the payload.
        found: u8,
    },
    /// A field held a value the deterministic model cannot represent.
    Value {
        /// Name of the rejected field.
        field: &'static str,
    },
}

impl fmt::Display for AudioSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("audio snapshot is truncated"),
            Self::TrailingBytes => formatter.write_str("audio snapshot has trailing bytes"),
            Self::Magic => formatter.write_str("audio snapshot magic mismatch"),
            Self::Version { found } => {
                write!(formatter, "audio snapshot version {found} is not supported")
            }
            Self::Value { field } => write!(formatter, "audio snapshot field {field} is invalid"),
        }
    }
}

impl Error for AudioSnapshotError {}

/// Maps a `chronofork_core` codec failure to the audio snapshot contract.
pub(crate) fn snapshot_error(error: CodecError) -> AudioSnapshotError {
    match error {
        CodecError::TrailingBytes { .. } => AudioSnapshotError::TrailingBytes,
        _ => AudioSnapshotError::Truncated,
    }
}

/// Decodes a canonical boolean snapshot byte.
pub(crate) fn decode_flag(value: u8, field: &'static str) -> Result<bool, AudioSnapshotError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(AudioSnapshotError::Value { field }),
    }
}
