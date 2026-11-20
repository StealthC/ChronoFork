//! Mega Drive audio devices.
//!
//! Phase 8.4 adds the YM2612 six-channel FM synthesiser ([`ym2612`]) and the
//! SN76489-compatible PSG ([`psg`]); the integer mixer and 48 kHz resampler are
//! the next task of the same phase. Nothing here talks to a host audio device:
//! the devices are deterministic integer models that the backend advances by
//! elapsed master cycles, and their sample functions are pure functions of
//! generated state.

pub mod psg;
pub mod ym2612;

pub use psg::Psg;
pub use ym2612::{EgState, EnvelopeView, Ym2612, YmBank, YmIrq, YmPort};
