//! `CB`-prefixed Z80 instructions: rotates, shifts, `BIT`, `SET` and `RES`.

use std::error::Error;

use chronofork_megadrive::z80::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_X, FLAG_Y, FLAG_Z, Z80, Z80Bus, Z80Error,
};

const RAM_SIZE: usize = 0x1_0000;
const DOCUMENTED_FLAGS: u8 = FLAG_S | FLAG_Z | FLAG_H | FLAG_PV | FLAG_N | FLAG_C;

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

fn documented_flags(z80: &Z80) -> u8 {
    z80.f() & DOCUMENTED_FLAGS
}

#[test]
fn rlc_and_rrc_carry_the_shifted_bit() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x81, 0xCB, 0x00]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.bc() >> 8, 0x03);
    assert_eq!(documented_flags(&z80), FLAG_PV | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x01, 0xCB, 0x08]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x80);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_C);
    Ok(())
}

#[test]
fn rl_and_rr_thread_the_carry_flag() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x37, 0x06, 0x00, 0xCB, 0x10, 0xCB, 0x18]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x01);
    assert_eq!(documented_flags(&z80), 0);
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x00);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_PV | FLAG_C);
    Ok(())
}

#[test]
fn shifts_match_the_documented_table() -> Result<(), Box<dyn Error>> {
    // (operation byte, input, expected result, expected carry)
    let cases: [(u8, u8, u8, bool); 4] = [
        (0x20, 0x81, 0x02, true),  // SLA
        (0x28, 0x81, 0xC0, true),  // SRA keeps the sign bit
        (0x30, 0x40, 0x81, false), // SLL shifts in a one
        (0x38, 0x81, 0x40, true),  // SRL
    ];
    for (opcode, input, expected, carry) in cases {
        let mut z80 = Z80::new();
        let mut bus = RamBus::new(&[0x06, input, 0xCB, opcode]);
        z80.step(&mut bus)?;
        z80.step(&mut bus)?;
        assert_eq!(z80.bc() >> 8, u16::from(expected), "opcode {opcode:#04x}");
        assert_eq!(z80.f() & FLAG_C != 0, carry, "opcode {opcode:#04x}");
    }
    Ok(())
}

#[test]
fn bit_7_of_register_sets_z() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x7F, 0xCB, 0x78]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.f() & FLAG_Z, FLAG_Z);
    assert_eq!(z80.f() & FLAG_S, 0);
    assert_eq!(z80.f() & FLAG_H, FLAG_H);
    assert_eq!(z80.f() & FLAG_N, 0);
    assert_eq!(z80.f() & FLAG_PV, FLAG_PV);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x80, 0xCB, 0x78]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.f() & FLAG_Z, 0);
    assert_eq!(z80.f() & FLAG_S, FLAG_S);
    assert_eq!(z80.f() & FLAG_PV, 0);
    Ok(())
}

#[test]
fn bit_tests_every_bit_of_a_register() -> Result<(), Box<dyn Error>> {
    for bit in 0_u8..8 {
        let mut z80 = Z80::new();
        z80.set_bc(u16::from(1_u8 << bit));
        let mut bus = RamBus::new(&[0xCB, 0x40 | (bit << 3) | 1]);
        z80.step(&mut bus)?;
        assert_eq!(z80.f() & FLAG_Z, 0, "bit {bit} set");

        let mut z80 = Z80::new();
        let mut bus = RamBus::new(&[0xCB, 0x40 | (bit << 3) | 1]);
        z80.step(&mut bus)?;
        assert_eq!(z80.f() & FLAG_Z, FLAG_Z, "bit {bit} clear");
    }
    Ok(())
}

#[test]
fn bit_copies_the_operand_into_the_undocumented_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_bc(0x0028); // bits 5 and 3 set
    let mut bus = RamBus::new(&[0xCB, 0x41]);
    z80.step(&mut bus)?;
    assert_eq!(z80.f() & FLAG_Y, FLAG_Y);
    assert_eq!(z80.f() & FLAG_X, FLAG_X);
    Ok(())
}

#[test]
fn bit_preserves_the_carry_flag() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xCB, 0x78]);
    z80.set_f(FLAG_C);
    z80.step(&mut bus)?;
    assert_eq!(z80.f() & FLAG_C, FLAG_C);
    Ok(())
}

#[test]
fn set_and_res_update_a_memory_cell() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_hl(0x1000);
    let mut bus = RamBus::new(&[0xCB, 0xC6, 0xCB, 0x86]);
    assert_eq!(z80.step(&mut bus)?.t_states, 15);
    assert_eq!(bus.read(0x1000), 0x01);
    assert_eq!(z80.step(&mut bus)?.t_states, 15);
    assert_eq!(bus.read(0x1000), 0x00);
    Ok(())
}

#[test]
fn set_and_res_do_not_change_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_f(FLAG_S | FLAG_Z | FLAG_H | FLAG_PV | FLAG_N | FLAG_C);
    let mut bus = RamBus::new(&[0xCB, 0xC0]);
    z80.step(&mut bus)?;
    assert_eq!(
        documented_flags(&z80),
        FLAG_S | FLAG_Z | FLAG_H | FLAG_PV | FLAG_N | FLAG_C
    );
    Ok(())
}

#[test]
fn bit_memory_sets_z_and_h_with_the_documented_cost() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_hl(0x1000);
    let mut bus = RamBus::new(&[0xCB, 0x7E]);
    bus.write(0x1000, 0x80);
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(z80.f() & FLAG_Z, 0);
    assert_eq!(z80.f() & FLAG_S, FLAG_S);
    assert_eq!(z80.f() & FLAG_H, FLAG_H);
    Ok(())
}

#[test]
fn shift_flags_include_parity_and_clear_half_carry() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x01, 0xCB, 0x38]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x00);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_PV | FLAG_C);
    Ok(())
}
