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

pub use mixer::{AudioMixer, OUTPUT_CHANNELS, OUTPUT_SAMPLE_RATE_HZ};
pub use psg::Psg;
pub use resample::{RationalResampler, ResampleError};
pub use ym2612::{EgState, EnvelopeView, Ym2612, YmBank, YmIrq, YmPort};
