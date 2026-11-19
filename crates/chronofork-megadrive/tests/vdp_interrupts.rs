#![allow(clippy::expect_used)]

//! VDP-owned VBlank/HBlank interrupt control.
//!
//! Register `0x00` bits 5 (VBlank) and 4 (HBlank) gate request generation. The
//! VDP owns the pending/acknowledge surface: a control-port status read reports
//! the requests but never acknowledges them, and acknowledging the delivered
//! source leaves the other request pending. VBlank (level 6) is delivered before
//! HBlank (level 4) and the 68000 `SR` mask still decides admission.

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, STATUS_VBLANK, STATUS_VINT_PENDING, Vdp, VdpError, VdpInterruptSource,
};
use chronofork_megadrive::{InterruptLevel, MegadriveFactory, interrupt_rom};

fn write_register(vdp: &mut Vdp, register: u8, value: u8) -> Result<(), VdpError> {
    vdp.write16(
        CONTROL_PORT_OFFSET,
        0x8000 | (u16::from(register) << 8) | u16::from(value),
    )
}

#[test]
fn disabled_interrupts_do_not_set_a_request() {
    let mut vdp = Vdp::new();
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);
    let pending = vdp.take_interrupts();
    assert!(!pending.vblank_pending);
    assert!(!pending.hblank_pending);

    write_register(&mut vdp, 0x00, 0x20).expect("enable VBlank");
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    assert!(vdp.take_interrupts().vblank_pending);
    assert!(!vdp.take_interrupts().hblank_pending);
}

#[test]
fn register_enable_bits_gate_each_source_independently() {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x00, 0x20).expect("VBlank only");

    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);
    let pending = vdp.take_interrupts();
    assert!(pending.vblank_pending, "bit 5 enables VBlank");
    assert!(!pending.hblank_pending, "bit 4 is clear");

    vdp.acknowledge(VdpInterruptSource::VBlank);
    write_register(&mut vdp, 0x00, 0x10).expect("HBlank only");
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);
    let pending = vdp.take_interrupts();
    assert!(!pending.vblank_pending);
    assert!(pending.hblank_pending, "bit 4 enables HBlank");
}

#[test]
fn a_status_read_reports_but_never_acknowledges_requests() {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x00, 0x30).expect("enable both");
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);

    let status = vdp.read16(CONTROL_PORT_OFFSET).expect("status read");
    let _ = status;
    let pending = vdp.take_interrupts();
    assert!(pending.vblank_pending, "a status read must not acknowledge");
    assert!(pending.hblank_pending, "a status read must not acknowledge");
}

#[test]
fn acknowledging_one_source_leaves_the_other_pending() {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x00, 0x30).expect("enable both");
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    vdp.request_interrupt(VdpInterruptSource::HBlank);

    // Coalescing: a second request while pending keeps one latch.
    vdp.request_interrupt(VdpInterruptSource::VBlank);
    assert!(vdp.take_interrupts().vblank_pending);

    vdp.acknowledge(VdpInterruptSource::VBlank);
    let pending = vdp.take_interrupts();
    assert!(!pending.vblank_pending);
    assert!(pending.hblank_pending);

    vdp.clear_interrupts();
    let pending = vdp.take_interrupts();
    assert!(!pending.vblank_pending && !pending.hblank_pending);
}

#[test]
fn enabled_vblank_delivers_level_6_and_rte_restores_the_mask() {
    let factory = MegadriveFactory::new(interrupt_rom(5)).expect("interrupt cartridge");
    let mut machine = factory.create_machine().expect("machine");

    let mut vblank_ran = false;
    for _ in 0..1_000_000 {
        machine.step(InputFrame::NEUTRAL).expect("step");
        if machine.inspect().expect("inspect").registers["d6"] >= 1 {
            vblank_ran = true;
            break;
        }
    }
    assert!(vblank_ran, "the enabled VBlank handler must run");

    // RTE returns to the loop and restores the lowered mask.
    for _ in 0..100 {
        if machine.cpu().sr() & 0x0700 == 0x0500 {
            break;
        }
        machine.step(InputFrame::NEUTRAL).expect("rte");
    }

    let state = machine.inspect().expect("inspect");
    assert_eq!(state.registers["d6"], 1, "one VBlank handler entry");
    assert_eq!(
        state.registers["vdp_status"] & u64::from(STATUS_VBLANK),
        u64::from(STATUS_VBLANK),
        "the VDP reports vertical blanking"
    );
    assert_eq!(
        state.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
        0,
        "delivery acknowledged the VBlank request"
    );
    assert_eq!(
        machine.cpu().sr() & 0x0700,
        0x0500,
        "RTE restored the lowered mask"
    );
    assert!(
        machine.interrupts().is_pending(InterruptLevel::HBlank),
        "the masked HBlank request is retained, not lost"
    );
}
