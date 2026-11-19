//! VDP line/dot position, H/V counters and master-cycle frame timing.
//!
//! The device advances on the master-cycle timeline owned by the backend. One
//! line is [`MASTER_CYCLES_PER_LINE`] master cycles; the active display is 224
//! (NTSC) or 240 (PAL) lines and VBlank covers the rest of the frame.
//!
//! **The counters are approximations, not dot-exact hardware values.** The H
//! counter follows the documented H40 progression scaled across the line; the V
//! counter follows the documented eight-bit readout; the H32/H40 mode selection
//! that would pick the other H progression arrives with register-controlled
//! display width, and interlace is not modelled. Phase 8.2 evidence already
//! records that H/V-counter timing is not claimed against a hardware oracle.
//!
//! The position is canonical state and must never be driven by host time.
//! Splitting an advance into chunks produces the same state as one advance.

use crate::cycles::{MASTER_CYCLES_PER_LINE, active_lines, lines_per_frame};
use crate::interrupts::VideoStandard;

use super::Vdp;

/// First master-cycle dot of HBlank in an H40 line.
///
/// 320 active pixels at eight master cycles each is 2560 master cycles; the
/// remaining 860 master cycles of the 3420-cycle line are horizontal blanking.
/// H32 mode would shorten the active interval and is not modelled yet, so this
/// is a documented H40 approximation.
pub(super) const HBLANK_START_MASTER_CYCLES: u16 = 2560;

/// Number of documented H40 H-counter units in one line.
///
/// The counter returns `0x00..=0xB6` (183 values) and `0xE4..=0xFF` (28
/// values); the HSYNC gap `0xB7..=0xE3` is never returned.
const H_COUNTER_H40_UNITS: usize = 183 + 28;

/// Documented H40 H-counter progression, one entry per unit.
///
/// Source: Charles MacDonald's measured H/V-counter progression, mirrored in
/// the Sega Mega Drive hardware notes (`H40: 0x00..=0xB6, 0xE4..=0xFF`; one H
/// unit is two pixels). A line's 3420 master cycles are scaled onto the 211
/// units. The table pins the documented ranges and the HSYNC gap, but the
/// transition points are an approximation: the counter is not dot-exact, the
/// real chip's pipeline latency is not modelled, and H32 selection is deferred.
const H_COUNTER_H40: [u8; H_COUNTER_H40_UNITS] = build_h_counter_h40();

const fn build_h_counter_h40() -> [u8; H_COUNTER_H40_UNITS] {
    let mut table = [0_u8; H_COUNTER_H40_UNITS];
    let mut index = 0;
    while index < 183 {
        table[index] = index as u8;
        index += 1;
    }
    let mut second = 0;
    while second < 28 {
        table[183 + second] = 0xE4 + second as u8;
        second += 1;
    }
    table
}

/// Result of one master-cycle advance.
///
/// `line` and `dot` are the position after the advance. `hblank` and `vblank`
/// report whether the advanced interval reached the horizontal-blanking tail or
/// the vertical-blanking lines respectively; at a line boundary the interval
/// that just completed still counts as having reached them. `frame_started` is
/// true when at least one frame boundary was crossed. The status register, by
/// contrast, always reflects the position after the advance.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VdpTick {
    /// Line within the frame after the advance.
    pub line: u16,
    /// Master-cycle dot within the line after the advance.
    pub dot: u16,
    /// Whether the interval reached the horizontal-blanking tail.
    pub hblank: bool,
    /// Whether the interval reached the vertical-blanking lines.
    pub vblank: bool,
    /// Whether at least one frame boundary was crossed.
    pub frame_started: bool,
}

impl Vdp {
    /// Returns the current line within the video frame.
    #[must_use]
    pub const fn line(&self) -> u16 {
        self.line
    }

    /// Returns the current master-cycle dot within the line.
    #[must_use]
    pub const fn dot(&self) -> u16 {
        self.dot
    }

    /// Returns how many complete frames have elapsed.
    #[must_use]
    pub const fn frame_index(&self) -> u64 {
        self.frame_index
    }

    /// Returns the video standard the device timing last advanced under.
    ///
    /// The reset value is NTSC. The owning backend passes the configured
    /// standard to [`Vdp::advance_master`], which is also what drives the PAL
    /// status bit.
    #[must_use]
    pub const fn video_standard(&self) -> VideoStandard {
        self.video
    }

    /// Advances the video timeline by `cycles` master cycles.
    ///
    /// The dot counter wraps at [`MASTER_CYCLES_PER_LINE`], the line counter
    /// wraps at the standard's frame length, `frame_index` counts completed
    /// frames and the VBlank status latch follows the active-line count. The
    /// advance is a pure function of the supplied cycle count and canonical
    /// state: chunking the same total gives identical results.
    ///
    /// Inputs beyond the representable timeline saturate deterministically
    /// instead of panicking.
    pub fn advance_master(&mut self, cycles: u64, video: VideoStandard) -> VdpTick {
        self.video = video;
        let line_cycles = u128::from(MASTER_CYCLES_PER_LINE);
        let frame_lines = u128::from(lines_per_frame(video));
        let active = u128::from(active_lines(video));
        let frame_cycles = line_cycles * frame_lines;

        let start = u128::from(self.line) * line_cycles + u128::from(self.dot);
        let end = start + u128::from(cycles);
        let end_lines = end / line_cycles;
        let end_dot = end % line_cycles;
        let frames = end / frame_cycles;

        let line = (end_lines % frame_lines) as u16;
        let dot = end_dot as u16;
        let hblank =
            end_lines > u128::from(self.line) || end_dot > u128::from(HBLANK_START_MASTER_CYCLES);
        let vblank = end_lines >= active;
        let frame_started = frames > 0;

        self.frame_index = self
            .frame_index
            .saturating_add(u64::try_from(frames).unwrap_or(u64::MAX));
        self.line = line;
        self.dot = dot;
        self.status_latches.vblank = line >= active_lines(video);
        VdpTick {
            line,
            dot,
            hblank,
            vblank,
            frame_started,
        }
    }

    /// Returns the current H counter value.
    ///
    /// The documented H40 progression is scaled across the line: one H unit is
    /// two pixels, so 3420 master cycles map onto 211 units. This is
    /// approximate and not dot-exact; see the module documentation.
    #[must_use]
    pub fn h_counter(&self) -> u8 {
        let units = u64::from(self.dot) * H_COUNTER_H40_UNITS as u64 / MASTER_CYCLES_PER_LINE;
        H_COUNTER_H40[units as usize]
    }

    /// Returns the current V counter value.
    ///
    /// NTSC counts `0x00..=0xEA` and then continues from `0xE5` so all 262
    /// lines fit the eight-bit readout; PAL returns the low eight bits of its
    /// nine-bit counter (the ninth bit belongs to interlace, which is not
    /// modelled). This is the documented readout approximation, not a
    /// dot-exact vertical position.
    #[must_use]
    pub fn v_counter(&self) -> u8 {
        match self.video {
            VideoStandard::Ntsc => {
                if self.line <= 0xEA {
                    self.line as u8
                } else {
                    (0xE5 + (self.line - 0xEB)) as u8
                }
            }
            VideoStandard::Pal => (self.line & 0xFF) as u8,
        }
    }

    /// Returns the combined HV-counter word as the port reads it.
    pub(super) fn hv_counter(&self) -> u16 {
        (u16::from(self.v_counter()) << 8) | u16::from(self.h_counter())
    }
}
