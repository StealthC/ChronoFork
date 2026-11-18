//! Register-file behaviour of the 68000 core: supervisor and user stacks.

use chronofork_megadrive::cpu::M68000;

#[test]
fn supervisor_and_user_stacks_are_separate() {
    let mut cpu = M68000::new();
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FF_0000); // supervisor stack
    cpu.set_sr(0x0700); // switch to user mode
    cpu.set_a7(0x0000_1000); // user stack
    assert_eq!(cpu.usp(), 0x0000_1000);
    assert_eq!(cpu.ssp(), 0x00FF_0000);
    cpu.set_sr(0x2700);
    assert_eq!(cpu.a7(), 0x00FF_0000);
}

#[test]
fn switching_modes_preserves_the_inactive_stack_pointer() {
    let mut cpu = M68000::new();
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FF_0000);
    cpu.set_sr(0x0700);
    cpu.set_a7(0x0000_1000);
    cpu.set_sr(0x2700);
    cpu.set_a7(0x00FE_0000);
    // The supervisor's A7 was saved into SSP, not into USP.
    assert_eq!(cpu.ssp(), 0x00FE_0000);
    assert_eq!(cpu.usp(), 0x0000_1000);
    cpu.set_sr(0x0700);
    assert_eq!(cpu.a7(), 0x0000_1000);
    assert_eq!(
        cpu.sr() & 0x2000,
        0,
        "user mode has the supervisor bit clear"
    );
}
