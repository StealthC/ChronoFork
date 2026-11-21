//! Z80 interrupt acceptance: IM 0/1/2, NMI, mask and `EI` delay, `HALT` wake.

use std::error::Error;

use chronofork_megadrive::z80::interrupts::{
    IM1_ACKNOWLEDGE_T_STATES, IM1_VECTOR_ADDRESS, IM2_ACKNOWLEDGE_T_STATES,
    NMI_ACKNOWLEDGE_T_STATES, NMI_VECTOR_ADDRESS,
};
use chronofork_megadrive::z80::{SNAPSHOT_LEN, Z80, Z80Bus, Z80Error, Z80Field};

const RAM_SIZE: usize = 0x1_0000;

#[derive(Debug)]
struct RamBus {
    ram: Vec<u8>,
}

impl RamBus {
    fn new(program: &[u8]) -> Self {
        let mut ram = vec![0_u8; RAM_SIZE];
        ram[..program.len()].copy_from_slice(program);
        Self { ram }
    }

    fn read(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }

    fn write(&mut self, address: u16, value: u8) {
        self.ram[usize::from(address)] = value;
    }
}

impl Z80Bus for RamBus {
    fn read8(&mut self, address: u16) -> Result<u8, Z80Error> {
        Ok(self.ram[usize::from(address)])
    }

    fn write8(&mut self, address: u16, value: u8) -> Result<(), Z80Error> {
        self.ram[usize::from(address)] = value;
        Ok(())
    }

    fn input(&mut self, _port: u16) -> Result<u8, Z80Error> {
        Ok(0xFF)
    }

    fn output(&mut self, _port: u16, _value: u8) -> Result<(), Z80Error> {
        Ok(())
    }
}

#[test]
fn im1_jumps_to_0x0038() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x00]);
    z80.set_iff1(true);
    z80.request_interrupt(0xFF);
    assert_eq!(z80.step(&mut bus)?.t_states, IM1_ACKNOWLEDGE_T_STATES);
    assert_eq!(z80.pc(), IM1_VECTOR_ADDRESS);
    assert_eq!(z80.sp(), 0xFFFE);
    assert_eq!(bus.read(0xFFFE), 0x00, "the interrupted PC is pushed");
    assert_eq!(bus.read(0xFFFF), 0x00);
    assert!(!z80.take_interrupt_pending(), "the request was consumed");
    Ok(())
}

#[test]
fn im2_uses_vector_table() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xED, 0x5E]);
    z80.step(&mut bus)?; // IM 2
    assert_eq!(z80.im(), 2);
    z80.set_i(0x20);
    z80.set_iff1(true);
    z80.request_interrupt(0x42);
    bus.write(0x2042, 0x34);
    bus.write(0x2043, 0x12);
    assert_eq!(z80.step(&mut bus)?.t_states, IM2_ACKNOWLEDGE_T_STATES);
    assert_eq!(z80.pc(), 0x1234);
    assert_eq!(bus.read(0xFFFE), 0x02, "the interrupted PC is pushed");
    Ok(())
}

#[test]
fn im0_executes_the_supplied_rst() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xED, 0x46]);
    z80.step(&mut bus)?; // IM 0
    assert_eq!(z80.im(), 0);
    z80.set_iff1(true);
    z80.request_interrupt(0xEF); // RST 28h
    assert_eq!(z80.step(&mut bus)?.t_states, 13);
    assert_eq!(z80.pc(), 0x0028);
    assert_eq!(bus.read(0xFFFE), 0x02, "RST pushes the interrupted PC");
    Ok(())
}

#[test]
fn nmi_saves_iff_state_and_restores() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xED, 0x45]);
    z80.set_iff1(true);
    z80.set_iff2(true);
    z80.request_nmi();
    assert_eq!(z80.step(&mut bus)?.t_states, NMI_ACKNOWLEDGE_T_STATES);
    assert_eq!(z80.pc(), NMI_VECTOR_ADDRESS);
    assert!(!z80.iff1(), "NMI masks maskable interrupts");
    assert!(z80.iff2(), "IFF2 keeps the previous mask");

    z80.set_pc(0);
    assert_eq!(z80.step(&mut bus)?.t_states, 14, "RETN");
    assert_eq!(z80.pc(), 0, "RETN returns to the interrupted instruction");
    assert!(z80.iff1(), "RETN restores IFF1 from IFF2");
    Ok(())
}

#[test]
fn masked_request_waits_for_iff1() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x00]);
    z80.request_interrupt(0x00);
    assert!(!z80.take_interrupt_pending(), "IFF1 is clear");
    assert_eq!(z80.step(&mut bus)?.t_states, 4, "a masked request runs NOP");
    assert_eq!(z80.pc(), 1);
    z80.set_iff1(true);
    assert!(z80.take_interrupt_pending(), "the latch survives the mask");
    assert!(!z80.take_interrupt_pending(), "taking consumes the request");
    Ok(())
}

#[test]
fn ei_delay_blocks_acceptance_for_one_instruction() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xFB, 0x00]);
    z80.step(&mut bus)?; // EI
    assert!(z80.ei_delay_pending());
    z80.request_interrupt(0xFF);
    assert!(!z80.take_interrupt_pending(), "EI delays acceptance");
    assert_eq!(
        z80.step(&mut bus)?.t_states,
        4,
        "the next instruction executes"
    );
    assert_eq!(z80.pc(), 2);
    assert_eq!(
        z80.step(&mut bus)?.t_states,
        IM1_ACKNOWLEDGE_T_STATES,
        "the following boundary accepts the request"
    );
    assert_eq!(z80.pc(), IM1_VECTOR_ADDRESS);
    Ok(())
}

#[test]
fn halt_wakes_on_interrupt() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x76]);
    z80.set_iff1(true);
    z80.step(&mut bus)?;
    assert!(z80.halted());
    z80.request_interrupt(0xFF);
    assert_eq!(z80.step(&mut bus)?.t_states, IM1_ACKNOWLEDGE_T_STATES);
    assert!(!z80.halted());
    assert_eq!(z80.pc(), IM1_VECTOR_ADDRESS);
    assert_eq!(bus.read(0xFFFE), 0x01, "the byte after HALT is pushed");
    Ok(())
}

#[test]
fn nmi_wakes_halt_while_maskable_interrupts_are_disabled() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x76, 0x00, 0xED, 0x45]);
    z80.set_iff1(false);
    z80.set_iff2(true);
    z80.step(&mut bus)?;
    assert!(z80.halted());
    z80.request_nmi();
    assert_eq!(z80.step(&mut bus)?.t_states, NMI_ACKNOWLEDGE_T_STATES);
    assert!(!z80.halted());
    assert_eq!(z80.pc(), NMI_VECTOR_ADDRESS);
    assert!(!z80.iff1());

    z80.set_pc(2);
    z80.step(&mut bus)?; // RETN
    assert!(z80.iff1());
    assert_eq!(z80.pc(), 1);
    Ok(())
}

#[test]
fn nmi_is_serviced_before_a_pending_maskable_request() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xED, 0x56]);
    z80.step(&mut bus)?; // IM 1
    z80.set_iff1(true);
    z80.request_interrupt(0xFF);
    z80.request_nmi();
    assert_eq!(z80.step(&mut bus)?.t_states, NMI_ACKNOWLEDGE_T_STATES);
    assert_eq!(z80.pc(), NMI_VECTOR_ADDRESS);
    assert!(!z80.iff1());

    z80.set_iff1(true);
    assert_eq!(z80.step(&mut bus)?.t_states, IM1_ACKNOWLEDGE_T_STATES);
    assert_eq!(
        z80.pc(),
        IM1_VECTOR_ADDRESS,
        "the maskable request was kept"
    );
    Ok(())
}

#[test]
fn pending_requests_survive_a_snapshot_round_trip() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xED, 0x56]);
    z80.step(&mut bus)?; // IM 1
    z80.set_iff1(false);
    z80.request_interrupt(0x00);
    let bytes = z80.snapshot_bytes();
    assert_eq!(bytes.len(), SNAPSHOT_LEN);

    let mut restored = Z80::new();
    restored.restore_bytes(&bytes)?;
    assert!(!restored.take_interrupt_pending(), "the mask is restored");
    restored.set_iff1(true);
    assert_eq!(restored.step(&mut bus)?.t_states, IM1_ACKNOWLEDGE_T_STATES);
    assert_eq!(restored.pc(), IM1_VECTOR_ADDRESS);

    let mut z80 = Z80::new();
    z80.request_nmi();
    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();
    restored.restore_bytes(&bytes)?;
    assert_eq!(restored.step(&mut bus)?.t_states, NMI_ACKNOWLEDGE_T_STATES);
    assert_eq!(restored.pc(), NMI_VECTOR_ADDRESS);
    Ok(())
}

#[test]
fn snapshot_rejects_non_canonical_request_latches() -> Result<(), Box<dyn Error>> {
    let z80 = Z80::new();
    let bytes = z80.snapshot_bytes();
    let mut restored = Z80::new();

    let mut bad_request = bytes.clone();
    bad_request[SNAPSHOT_LEN - 3] = 2;
    assert_eq!(
        restored.restore_bytes(&bad_request),
        Err(Z80Error::SnapshotField {
            field: Z80Field::InterruptRequest
        })
    );

    let mut bad_nmi = bytes;
    bad_nmi[SNAPSHOT_LEN - 1] = 2;
    assert_eq!(
        restored.restore_bytes(&bad_nmi),
        Err(Z80Error::SnapshotField {
            field: Z80Field::NmiRequest
        })
    );
    Ok(())
}
