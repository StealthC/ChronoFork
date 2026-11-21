//! Master-clock time base for the Mega Drive.
//!
//! Canonical emulated time is an integer master-clock cycle. Every other
//! device clock divides the master clock by an integer ratio: the 68000 runs
//! at one seventh of it, so one 68000 cycle is [`MASTER_CYCLES_PER_68000_CYCLE`]
//! master cycles, and a scanline lasts [`MASTER_CYCLES_PER_LINE`] master cycles.
//! Frame length and frame rate follow from the master clock and the line count,
//! so no floating point or host clock enters canonical state.
//!
//! Constants are documented from Sega's Mega Drive hardware documentation and
//! cross-checked against the Exodus reference implementation. They are the
//! modelled nominal timing, not a measurement of a particular console.

use crate::interrupts::VideoStandard;

/// Master-clock cycles in one 68000 clock cycle.
///
/// The 68000 clock is the master clock divided by seven
/// (`VCLK = MCLK / 7` in Sega's documentation), so every 68000 cycle the
/// backend advances corresponds to seven master cycles.
pub const MASTER_CYCLES_PER_68000_CYCLE: u64 = 7;

/// Master-clock cycles in one Z80 T-state.
///
/// The Z80 clock is the master clock divided by fifteen, so one Z80 T-state is
/// fifteen master cycles. [`crate::z80::Z80Outcome::master_cycles`] converts an
/// instruction's documented T-state count with this ratio; the Z80 core itself
/// counts T-states only.
pub const MASTER_CYCLES_PER_Z80_T_STATE: u64 = 15;

/// Master-clock cycles in one scanline.
///
/// Sega's documentation gives 3420 master cycles per active or blanking
/// scanline on both video standards; the 68000 therefore has roughly 488
/// clock cycles per line, and the exact per-instruction split belongs to the
/// CPU cycle table, not to this time base.
pub const MASTER_CYCLES_PER_LINE: u64 = 3420;

/// NTSC scanlines in one frame, including vertical blanking.
///
/// Source: Sega Mega Drive hardware documentation (NTSC video timing);
/// consistent with the master-cycle scheduler and the Exodus reference
/// implementation.
pub const NTSC_LINES_PER_FRAME: u16 = 262;

/// PAL scanlines in one frame, including vertical blanking.
///
/// Source: Sega Mega Drive hardware documentation (PAL video timing);
/// consistent with the master-cycle scheduler and the Exodus reference
/// implementation.
pub const PAL_LINES_PER_FRAME: u16 = 313;

/// NTSC active display scanlines.
///
/// Source: Sega Mega Drive hardware documentation (NTSC visible picture).
pub const NTSC_ACTIVE_LINES: u16 = 224;

/// PAL active display scanlines.
///
/// Source: Sega Mega Drive hardware documentation (PAL visible picture).
pub const PAL_ACTIVE_LINES: u16 = 240;

/// NTSC master-clock frequency in hertz.
///
/// Source: Sega Mega Drive hardware documentation (53.693175 MHz); the value
/// cited by the Exodus reference implementation.
pub const MASTER_CLOCK_HZ_NTSC: u64 = 53_693_175;

/// PAL master-clock frequency in hertz.
///
/// Source: Sega Mega Drive hardware documentation (53.203424 MHz); the value
/// cited by the Exodus reference implementation.
pub const MASTER_CLOCK_HZ_PAL: u64 = 53_203_424;

/// Resolution, in millihertz, at which the NTSC frame rate is published.
///
/// Sega documents the NTSC frame rate as 59.92 Hz (hundredths of a hertz).
const NTSC_FRAME_RATE_RESOLUTION_MILLIHERTZ: u64 = 10;

/// Resolution, in millihertz, at which the PAL frame rate is published.
///
/// The documented PAL rate is 49.701 Hz (millihertz resolution).
const PAL_FRAME_RATE_RESOLUTION_MILLIHERTZ: u64 = 1;

/// Returns the number of scanlines in one frame, including blanking.
#[must_use]
pub const fn lines_per_frame(video: VideoStandard) -> u16 {
    match video {
        VideoStandard::Ntsc => NTSC_LINES_PER_FRAME,
        VideoStandard::Pal => PAL_LINES_PER_FRAME,
    }
}

/// Returns the number of active display scanlines.
#[must_use]
pub const fn active_lines(video: VideoStandard) -> u16 {
    match video {
        VideoStandard::Ntsc => NTSC_ACTIVE_LINES,
        VideoStandard::Pal => PAL_ACTIVE_LINES,
    }
}

/// Returns the nominal frame rate in millihertz for `video`.
///
/// The rate is derived from the master clock and the frame length,
/// `MASTER_CLOCK_HZ * 1000 / (MASTER_CYCLES_PER_LINE * lines_per_frame)`.
/// For NTSC that derivation gives 59_922 millihertz (59.9227 Hz), which Sega
/// publishes as 59.92 Hz; the result is reported at that published
/// hundredth-of-a-hertz resolution. The PAL derivation gives 49_701 millihertz
/// (49.7014 Hz), the published value at millihertz resolution.
#[must_use]
pub fn frame_rate_millihertz(video: VideoStandard) -> u64 {
    let clock_hz = match video {
        VideoStandard::Ntsc => MASTER_CLOCK_HZ_NTSC,
        VideoStandard::Pal => MASTER_CLOCK_HZ_PAL,
    };
    let frame_cycles = MASTER_CYCLES_PER_LINE * u64::from(lines_per_frame(video));
    let derived = clock_hz * 1000 / frame_cycles;
    let resolution = match video {
        VideoStandard::Ntsc => NTSC_FRAME_RATE_RESOLUTION_MILLIHERTZ,
        VideoStandard::Pal => PAL_FRAME_RATE_RESOLUTION_MILLIHERTZ,
    };
    derived / resolution * resolution
}

#[cfg(test)]
mod tests {
    use crate::interrupts::VideoStandard;

    use super::{
        MASTER_CLOCK_HZ_NTSC, MASTER_CLOCK_HZ_PAL, MASTER_CYCLES_PER_68000_CYCLE,
        MASTER_CYCLES_PER_LINE, MASTER_CYCLES_PER_Z80_T_STATE, NTSC_ACTIVE_LINES,
        NTSC_LINES_PER_FRAME, PAL_ACTIVE_LINES, PAL_LINES_PER_FRAME, active_lines,
        frame_rate_millihertz, lines_per_frame,
    };

    #[test]
    fn frame_rate_derives_from_clock_and_lines() {
        assert_eq!(frame_rate_millihertz(VideoStandard::Ntsc), 59_920);
        assert_eq!(frame_rate_millihertz(VideoStandard::Pal), 49_701);
        assert_eq!(lines_per_frame(VideoStandard::Ntsc), 262);
        assert_eq!(active_lines(VideoStandard::Pal), 240);
    }

    #[test]
    fn master_cycle_constants_match_the_documented_ratios() {
        assert_eq!(MASTER_CYCLES_PER_68000_CYCLE, 7);
        assert_eq!(MASTER_CYCLES_PER_Z80_T_STATE, 15);
        assert_eq!(MASTER_CYCLES_PER_LINE, 3420);
        assert_eq!(NTSC_LINES_PER_FRAME, 262);
        assert_eq!(PAL_LINES_PER_FRAME, 313);
        assert_eq!(NTSC_ACTIVE_LINES, 224);
        assert_eq!(PAL_ACTIVE_LINES, 240);
        assert_eq!(MASTER_CLOCK_HZ_NTSC, 53_693_175);
        assert_eq!(MASTER_CLOCK_HZ_PAL, 53_203_424);
    }

    #[test]
    fn line_and_active_line_helpers_cover_both_standards() {
        assert_eq!(lines_per_frame(VideoStandard::Pal), 313);
        assert_eq!(active_lines(VideoStandard::Ntsc), 224);
    }
}
