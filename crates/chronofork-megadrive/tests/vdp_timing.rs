#![allow(clippy::expect_used)]

//! VDP status, H/V counters and master-cycle line timing.
//!
//! The device advances on the backend's master-cycle timeline. One line is
//! 3420 master cycles; the active display is 224 (NTSC) or 240 (PAL) lines.
//! HBlank is the tail of each line and VBlank starts at the active-line count.
//!
//! **These values are approximate, not dot-exact.** The H counter follows the
//! documented H40 progression (0x00..=0xB6, then 0xE4..=0xFF) scaled across the
//! line; the V counter follows the documented eight-bit readout. Interlace and
//! H32/H40 register selection are not modelled, so no consumer may treat these
//! counters as a hardware oracle.

use chronofork_megadrive::VideoStandard;
use chronofork_megadrive::cycles::MASTER_CYCLES_PER_LINE;
use chronofork_megadrive::vdp::{
    HV_COUNTER_OFFSET, STATUS_FIFO_EMPTY, STATUS_HBLANK, STATUS_PAL, STATUS_VBLANK,
    STATUS_VINT_PENDING, Vdp, VdpTick,
};

/// First master-cycle dot of HBlank in an H40 line: 320 active pixels at eight
/// master cycles each.
const HBLANK_START: u64 = 2560;

#[test]
fn status_and_counters_follow_the_line_position() {
    let mut vdp = Vdp::new();
    let tick = vdp.advance_master(MASTER_CYCLES_PER_LINE, VideoStandard::Ntsc);
    assert_eq!(tick.line, 1);
    assert_eq!(tick.dot, 0);
    assert!(
        tick.hblank,
        "a full line includes the line-end horizontal blanking interval"
    );
    assert!(!tick.vblank);
    assert!(!tick.frame_started);

    let vblank = vdp.advance_master(MASTER_CYCLES_PER_LINE * 223, VideoStandard::Ntsc);
    assert!(vblank.vblank);
    assert_eq!(vblank.line, 224);
    assert_eq!(vdp.line(), 224);
    assert_eq!(vdp.v_counter(), 224);
}

#[test]
fn h_counter_follows_the_documented_h40_progression() {
    let mut vdp = Vdp::new();
    assert_eq!(vdp.h_counter(), 0x00);

    // 1000 * 211 / 3420 = 61 = 0x3D (first documented range).
    vdp.advance_master(1000, VideoStandard::Ntsc);
    assert_eq!(vdp.dot(), 1000);
    assert_eq!(vdp.h_counter(), 0x3D);

    // 2900 * 211 / 3420 = 178 = 0xB2 (last value before the HSYNC gap).
    vdp.advance_master(1900, VideoStandard::Ntsc);
    assert_eq!(vdp.h_counter(), 0xB2);

    // 3300 * 211 / 3420 = 203 -> 0xE4 + 20 = 0xF8 (second documented range).
    vdp.advance_master(400, VideoStandard::Ntsc);
    assert_eq!(vdp.h_counter(), 0xF8);

    // One more line wraps the counter back to 0x00.
    vdp.advance_master(120, VideoStandard::Ntsc);
    assert_eq!(vdp.line(), 1);
    assert_eq!(vdp.h_counter(), 0x00);
}

#[test]
fn h_counter_never_returns_the_hsync_gap() {
    let mut vdp = Vdp::new();
    for _ in 0..MASTER_CYCLES_PER_LINE {
        vdp.advance_master(1, VideoStandard::Ntsc);
        let h = vdp.h_counter();
        assert!(
            !(0xB7..=0xE3).contains(&h),
            "H counter 0x{h:02X} must not be in the documented HSYNC gap"
        );
    }
    // Both documented ranges are reachable.
    let mut early = Vdp::new();
    early.advance_master(16, VideoStandard::Ntsc);
    assert!(early.h_counter() <= 0xB6);
    let mut late = Vdp::new();
    late.advance_master(3400, VideoStandard::Ntsc);
    assert!(late.h_counter() >= 0xE4);
}

#[test]
fn v_counter_uses_the_documented_eight_bit_readout() {
    let mut ntsc = Vdp::new();
    ntsc.advance_master(MASTER_CYCLES_PER_LINE * 224, VideoStandard::Ntsc);
    assert_eq!(ntsc.line(), 224);
    assert_eq!(ntsc.v_counter(), 224);

    // NTSC counts 0x00..=0xEA, then continues from 0xE5 to cover 262 lines.
    let mut late = Vdp::new();
    late.advance_master(MASTER_CYCLES_PER_LINE * 235, VideoStandard::Ntsc);
    assert_eq!(late.line(), 235);
    assert_eq!(late.v_counter(), 0xE5);
    late.advance_master(MASTER_CYCLES_PER_LINE * 26, VideoStandard::Ntsc);
    assert_eq!(late.line(), 261);
    assert_eq!(late.v_counter(), 0xFF);

    let mut pal = Vdp::new();
    pal.advance_master(MASTER_CYCLES_PER_LINE * 240, VideoStandard::Pal);
    assert_eq!(pal.line(), 240);
    assert_eq!(pal.v_counter(), 240);
    pal.advance_master(MASTER_CYCLES_PER_LINE * 72, VideoStandard::Pal);
    assert_eq!(pal.line(), 312);
    assert_eq!(pal.v_counter(), 0x38);
}

#[test]
fn frame_wraps_at_the_standard_line_count() {
    let mut ntsc = Vdp::new();
    let tick = ntsc.advance_master(MASTER_CYCLES_PER_LINE * 262, VideoStandard::Ntsc);
    assert!(tick.frame_started);
    assert_eq!(tick.line, 0);
    assert_eq!(tick.dot, 0);
    assert_eq!(ntsc.frame_index(), 1);

    let mut pal = Vdp::new();
    let tick = pal.advance_master(MASTER_CYCLES_PER_LINE * 313, VideoStandard::Pal);
    assert!(tick.frame_started);
    assert_eq!(tick.line, 0);
    assert_eq!(pal.frame_index(), 1);

    let mut early = Vdp::new();
    let tick = early.advance_master(MASTER_CYCLES_PER_LINE * 261, VideoStandard::Ntsc);
    assert!(!tick.frame_started);
    assert_eq!(tick.line, 261);
    assert_eq!(early.frame_index(), 0);
}

#[test]
fn advancing_in_chunks_is_equivalent_to_one_advance() {
    let mut single = Vdp::new();
    let tick = single.advance_master(MASTER_CYCLES_PER_LINE * 300 + 123, VideoStandard::Ntsc);
    let mut chunked = Vdp::new();
    for _ in 0..300 {
        chunked.advance_master(MASTER_CYCLES_PER_LINE, VideoStandard::Ntsc);
    }
    chunked.advance_master(123, VideoStandard::Ntsc);

    assert_eq!(single.line(), chunked.line());
    assert_eq!(single.dot(), chunked.dot());
    assert_eq!(single.frame_index(), chunked.frame_index());
    assert_eq!(single.h_counter(), chunked.h_counter());
    assert_eq!(single.v_counter(), chunked.v_counter());
    assert_eq!(single.status(), chunked.status());
    assert_eq!(single.snapshot_bytes(), chunked.snapshot_bytes());

    // The single advance crossed a frame (300 lines > 262).
    assert_eq!(tick.line, 300 - 262);
    assert_eq!(tick.dot, 123);
    assert!(tick.frame_started);
}

#[test]
fn status_reports_pal_hblank_vblank_and_vint_from_device_state() {
    let mut vdp = Vdp::new();
    assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
    vdp.set_vint_pending(true);
    assert_eq!(vdp.status(), STATUS_FIFO_EMPTY | STATUS_VINT_PENDING);
    vdp.set_vint_pending(false);

    vdp.advance_master(HBLANK_START, VideoStandard::Ntsc);
    assert_eq!(vdp.dot() as u64, HBLANK_START);
    assert_eq!(vdp.status() & STATUS_HBLANK, STATUS_HBLANK);
    assert_eq!(vdp.status() & STATUS_VBLANK, 0);

    vdp.advance_master(MASTER_CYCLES_PER_LINE * 224, VideoStandard::Ntsc);
    assert_eq!(vdp.line(), 224);
    assert_eq!(vdp.dot() as u64, HBLANK_START);
    let status = vdp.status();
    assert_eq!(status & STATUS_VBLANK, STATUS_VBLANK);
    assert_eq!(status & STATUS_HBLANK, STATUS_HBLANK);
    assert_eq!(status & STATUS_FIFO_EMPTY, STATUS_FIFO_EMPTY);

    let mut pal = Vdp::new();
    pal.advance_master(0, VideoStandard::Pal);
    assert_eq!(pal.status() & STATUS_PAL, STATUS_PAL);
    assert_eq!(pal.status() & STATUS_VBLANK, 0);
}

#[test]
fn vblank_status_clears_when_the_next_frame_starts() {
    let mut vdp = Vdp::new();
    vdp.advance_master(MASTER_CYCLES_PER_LINE * 224, VideoStandard::Ntsc);
    assert_eq!(vdp.status() & STATUS_VBLANK, STATUS_VBLANK);
    vdp.advance_master(MASTER_CYCLES_PER_LINE * 38, VideoStandard::Ntsc);
    assert_eq!(vdp.line(), 0);
    assert_eq!(vdp.status() & STATUS_VBLANK, 0);
}

#[test]
fn hv_counter_port_returns_the_v_and_h_counters() {
    let mut vdp = Vdp::new();
    assert_eq!(vdp.read16(HV_COUNTER_OFFSET).expect("hv read"), 0);

    vdp.advance_master(MASTER_CYCLES_PER_LINE * 224, VideoStandard::Ntsc);
    assert_eq!(vdp.read16(HV_COUNTER_OFFSET).expect("hv read"), 0xE000);
    assert_eq!(vdp.read8(HV_COUNTER_OFFSET).expect("v byte"), 0xE0);
    assert_eq!(vdp.read8(HV_COUNTER_OFFSET + 1).expect("h byte"), 0x00);

    // dot 3000 -> H 0xE6; line 224 -> V 0xE0.
    vdp.advance_master(3000, VideoStandard::Ntsc);
    assert_eq!(vdp.read16(HV_COUNTER_OFFSET).expect("hv read"), 0xE0E6);
}

#[test]
fn a_zero_advance_reports_the_current_position() {
    let mut vdp = Vdp::new();
    vdp.advance_master(4000, VideoStandard::Ntsc);
    let tick = vdp.advance_master(0, VideoStandard::Ntsc);
    assert_eq!(tick.line, 1);
    assert_eq!(tick.dot, 580);
    assert!(!tick.hblank);
    assert!(!tick.vblank);
    assert!(!tick.frame_started);
    assert_eq!(vdp.line(), 1);
}

#[test]
fn snapshot_round_trips_timing_state_and_video_standard() {
    let mut vdp = Vdp::new();
    vdp.advance_master(MASTER_CYCLES_PER_LINE * 240 + 3000, VideoStandard::Pal);
    assert_eq!(vdp.line(), 240);
    assert_eq!(vdp.dot(), 3000);
    let snapshot = vdp.snapshot_bytes();

    let mut restored = Vdp::new();
    restored.restore_bytes(&snapshot).expect("restore snapshot");
    assert_eq!(restored.line(), 240);
    assert_eq!(restored.dot(), 3000);
    assert_eq!(restored.frame_index(), vdp.frame_index());
    assert_eq!(restored.h_counter(), vdp.h_counter());
    assert_eq!(restored.v_counter(), vdp.v_counter());
    assert_eq!(restored.status(), vdp.status());
    assert_eq!(restored.status() & STATUS_PAL, STATUS_PAL);
    assert_eq!(restored.snapshot_bytes(), snapshot);
}

#[test]
fn tick_values_are_plain_copyable_state() {
    let mut vdp = Vdp::new();
    let tick = vdp.advance_master(MASTER_CYCLES_PER_LINE + 1, VideoStandard::Ntsc);
    let copied: VdpTick = tick;
    assert_eq!(copied, tick);
    assert_eq!(copied.line, 1);
    assert_eq!(copied.dot, 1);
}
