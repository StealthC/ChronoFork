//! Z80 state, reset, T-state time base and versioned snapshot behaviour.

use std::error::Error;

use chronofork_megadrive::z80::{
    SNAPSHOT_LEN, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, Z80, Z80Bus, Z80Error, Z80Field,
};

/// Minimal instruction-stream bus: unwritten addresses read as `NOP`.
#[derive(Debug, Default)]
struct ProgramBus {
    bytes: Vec<u8>,
}

impl ProgramBus {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
        }
    }
}

impl Z80Bus for ProgramBus {
    fn read8(&mut self, address: u16) -> Result<u8, Z80Error> {
        Ok(self.bytes.get(usize::from(address)).copied().unwrap_or(0))
    }

    fn write8(&mut self, _address: u16, _value: u8) -> Result<(), Z80Error> {
        Ok(())
    }

    fn input(&mut self, _port: u16) -> Result<u8, Z80Error> {
        Ok(0xFF)
    }

    fn output(&mut self, _port: u16, _value: u8) -> Result<(), Z80Error> {
        Ok(())
    }
}

/// Snapshot byte offsets of the fields validated by `restore_bytes`.
const OFFSET_VERSION: usize = 8;
const OFFSET_IFF1: usize = 35;
const OFFSET_IFF2: usize = 36;
const OFFSET_IM: usize = 37;
const OFFSET_HALTED: usize = 38;
const OFFSET_EI_DELAY: usize = 39;

fn program(bytes: &[u8]) -> (Z80, ProgramBus) {
    (Z80::new(), ProgramBus::new(bytes))
}

#[test]
fn reset_clears_registers_and_snapshot_round_trips() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_pc(0x1234);
    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();
    restored.restore_bytes(&bytes)?;
    assert_eq!(restored.pc(), 0x1234);
    assert!(!restored.iff1());
    Ok(())
}

#[test]
fn reset_clears_the_complete_control_state() -> Result<(), Box<dyn Error>> {
    let mut bus = ProgramBus::default();
    let mut z80 = Z80::new();
    z80.set_pc(0x1234);
    z80.set_sp(0x5678);
    z80.set_ix(0x9ABC);
    z80.set_iy(0xDEF0);
    z80.set_i(0x42);
    z80.set_r(0x81);
    z80.set_iff1(true);
    z80.set_iff2(true);
    z80.set_a(0xAA);
    z80.set_f(0xFF);
    z80.reset(&mut bus);
    assert_eq!(z80, Z80::new());
    assert_eq!(z80.pc(), 0);
    assert_eq!(z80.af(), 0);
    assert_eq!(z80.ix(), 0);
    assert_eq!(z80.iy(), 0);
    assert_eq!(z80.im(), 0);
    assert!(!z80.halted());
    assert!(!z80.ei_delay_pending());
    Ok(())
}

#[test]
fn snapshot_has_a_fixed_versioned_layout() -> Result<(), Box<dyn Error>> {
    let (z80, _) = program(&[]);
    let bytes = z80.snapshot_bytes();
    assert_eq!(bytes.len(), SNAPSHOT_LEN);
    assert_eq!(&bytes[..SNAPSHOT_MAGIC.len()], &SNAPSHOT_MAGIC);
    assert_eq!(bytes[OFFSET_VERSION], SNAPSHOT_VERSION);
    Ok(())
}

#[test]
fn snapshot_round_trips_every_modelled_field() -> Result<(), Box<dyn Error>> {
    // Arrange the alternate set with `EXX`, then overwrite the current set.
    let (mut z80, mut bus) = program(&[0xD9]);
    z80.set_af(0x1122);
    z80.set_bc(0x3344);
    z80.set_de(0x5566);
    z80.set_hl(0x7788);
    z80.step(&mut bus)?;
    z80.set_af(0xABCD);
    z80.set_bc(0xBCDE);
    z80.set_de(0xCDEF);
    z80.set_hl(0xDEF0);
    z80.set_ix(0x1234);
    z80.set_iy(0x5678);
    z80.set_sp(0x9ABC);
    z80.set_i(0x5A);
    z80.set_r(0xA5);
    z80.set_iff1(true);
    z80.set_iff2(false);

    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();
    restored.restore_bytes(&bytes)?;
    assert_eq!(restored.snapshot_bytes(), bytes);
    assert_eq!(restored.af(), 0xABCD);
    assert_eq!(restored.bc(), 0xBCDE);
    assert_eq!(restored.de(), 0xCDEF);
    assert_eq!(restored.hl(), 0xDEF0);
    assert_eq!(restored.ix(), 0x1234);
    assert_eq!(restored.iy(), 0x5678);
    assert_eq!(restored.sp(), 0x9ABC);
    assert_eq!(restored.i(), 0x5A);
    assert_eq!(restored.r(), 0xA5);
    assert!(restored.iff1());
    assert!(!restored.iff2());

    // `EXX` exposes the alternate set that was arranged before the snapshot.
    restored.set_pc(0);
    let mut swap_bus = ProgramBus::new(&[0xD9]);
    restored.step(&mut swap_bus)?;
    assert_eq!(restored.af(), 0xABCD, "EXX does not swap AF");
    assert_eq!(restored.bc(), 0x3344);
    assert_eq!(restored.de(), 0x5566);
    assert_eq!(restored.hl(), 0x7788);
    Ok(())
}

#[test]
fn snapshot_round_trips_interrupt_mode_halt_and_ei_delay() -> Result<(), Box<dyn Error>> {
    // EI; NOP; IM 2; HALT
    let (mut z80, mut bus) = program(&[0xFB, 0x00, 0xED, 0x5E, 0x76]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.im(), 2);
    assert!(!z80.ei_delay_pending());
    let t_states = z80.step(&mut bus)?.t_states;
    assert_eq!(t_states, 4);
    assert!(z80.halted());

    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();
    restored.restore_bytes(&bytes)?;
    assert_eq!(restored.snapshot_bytes(), bytes);
    assert_eq!(restored.im(), 2);
    assert!(restored.iff1());
    assert!(restored.iff2());
    assert!(restored.halted());
    assert!(!restored.ei_delay_pending());
    Ok(())
}

#[test]
fn snapshot_rejects_magic_version_and_length_errors() -> Result<(), Box<dyn Error>> {
    let (z80, _) = program(&[]);
    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();

    let mut wrong_magic = bytes.clone();
    wrong_magic[0] = b'X';
    assert_eq!(
        restored.restore_bytes(&wrong_magic),
        Err(Z80Error::SnapshotMagic)
    );

    let mut wrong_version = bytes.clone();
    wrong_version[OFFSET_VERSION] = SNAPSHOT_VERSION + 1;
    assert_eq!(
        restored.restore_bytes(&wrong_version),
        Err(Z80Error::SnapshotVersion {
            found: SNAPSHOT_VERSION + 1
        })
    );

    assert!(matches!(
        restored.restore_bytes(&bytes[..SNAPSHOT_LEN - 1]),
        Err(Z80Error::Snapshot(_))
    ));

    let mut extended = bytes.clone();
    extended.push(0);
    assert!(matches!(
        restored.restore_bytes(&extended),
        Err(Z80Error::Snapshot(_))
    ));
    Ok(())
}

#[test]
fn snapshot_rejects_non_canonical_fields() -> Result<(), Box<dyn Error>> {
    let (z80, _) = program(&[]);
    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();

    for (offset, field) in [
        (OFFSET_IFF1, Z80Field::Iff1),
        (OFFSET_IFF2, Z80Field::Iff2),
        (OFFSET_HALTED, Z80Field::Halted),
        (OFFSET_EI_DELAY, Z80Field::EiDelay),
    ] {
        let mut corrupt = bytes.clone();
        corrupt[offset] = 2;
        assert_eq!(
            restored.restore_bytes(&corrupt),
            Err(Z80Error::SnapshotField { field })
        );
    }

    let mut corrupt_mode = bytes.clone();
    corrupt_mode[OFFSET_IM] = 3;
    assert_eq!(
        restored.restore_bytes(&corrupt_mode),
        Err(Z80Error::SnapshotField {
            field: Z80Field::InterruptMode
        })
    );
    Ok(())
}

#[test]
fn rejected_snapshot_leaves_the_core_untouched() -> Result<(), Box<dyn Error>> {
    let (z80, _) = program(&[]);
    let mut restored = Z80::new();
    restored.set_pc(0x4321);
    let mut corrupt = z80.snapshot_bytes();
    corrupt[OFFSET_IM] = 7;
    assert!(restored.restore_bytes(&corrupt).is_err());
    assert_eq!(restored.pc(), 0x4321);
    Ok(())
}

#[test]
fn nop_costs_four_t_states_and_converts_to_master_cycles() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0x00]);
    let outcome = z80.step(&mut bus)?;
    assert_eq!(outcome.t_states, 4);
    assert_eq!(outcome.master_cycles(), 60);
    assert_eq!(z80.pc(), 1);
    Ok(())
}

#[test]
fn r_increments_once_per_m1_fetch() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0x00, 0xCB, 0x00, 0xDD, 0x00]);
    z80.step(&mut bus)?;
    assert_eq!(z80.r(), 1, "one opcode fetch");

    z80.step(&mut bus)?;
    assert_eq!(z80.r(), 3, "CB prefix and opcode are two M1 cycles");

    z80.step(&mut bus)?;
    assert_eq!(z80.r(), 5, "DD prefix and opcode are two M1 cycles");
    Ok(())
}

#[test]
fn r_keeps_its_top_bit_across_increments() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0x00, 0x00]);
    z80.set_r(0xFF);
    z80.step(&mut bus)?;
    assert_eq!(z80.r(), 0x80);
    z80.step(&mut bus)?;
    assert_eq!(z80.r(), 0x81);
    Ok(())
}

#[test]
fn halt_reports_four_t_states_without_fetching() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0x76, 0x00]);
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert!(z80.halted());
    assert_eq!(z80.pc(), 1);
    let refresh_before = z80.r();
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.pc(), 1, "a halted core does not fetch");
    assert_eq!(z80.r(), refresh_before);
    Ok(())
}

#[test]
fn ei_sets_iff_and_delays_acceptance_by_one_instruction() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0xFB, 0x00]);
    z80.step(&mut bus)?;
    assert!(z80.iff1());
    assert!(z80.iff2());
    assert!(
        z80.ei_delay_pending(),
        "EI delays acceptance by one instruction"
    );

    z80.step(&mut bus)?;
    assert!(z80.iff1());
    assert!(
        !z80.ei_delay_pending(),
        "the following instruction releases the delay"
    );
    Ok(())
}

#[test]
fn di_clears_both_interrupt_flip_flops() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0xFB, 0xF3, 0xFB, 0xF3]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert!(!z80.iff1());
    assert!(!z80.iff2());
    assert!(!z80.ei_delay_pending());
    Ok(())
}

#[test]
fn master_cycles_per_t_state_is_the_documented_fifteen() -> Result<(), Box<dyn Error>> {
    let (mut z80, mut bus) = program(&[0xCD, 0x10, 0x00]);
    let outcome = z80.step(&mut bus)?;
    assert_eq!(outcome.t_states, 17);
    assert_eq!(outcome.master_cycles(), 17 * 15);
    Ok(())
}
