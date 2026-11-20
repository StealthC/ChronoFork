//! Mega Drive audio devices.
//!
//! Phase 8.4 adds the YM2612 six-channel FM synthesiser first
//! ([`ym2612`]); the SN76489-compatible PSG, the mixer and the 48 kHz
//! resampler are separate tasks of the same phase. Nothing here talks to a host
//! audio device: the devices are deterministic integer models that the backend
//! advances by elapsed master cycles, and [`Ym2612::sample`] is a pure function
//! of generated state.

pub mod ym2612;

pub use ym2612::{EgState, EnvelopeView, Ym2612, YmBank, YmIrq, YmPort};
