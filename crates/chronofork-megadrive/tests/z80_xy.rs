//! `DD`/`FD` prefixes: `IX`/`IY` and displacement addressing.

use std::error::Error;

use chronofork_megadrive::z80::{FLAG_C, FLAG_H, FLAG_S, FLAG_Z, Z80, Z80Bus, Z80Error};

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
fn ld_ix_displacement_reads_expected_cell() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x7E, 0x05]);
    z80.set_ix(0x1000);
    bus.write(0x1005, 0x42);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(z80.a(), 0x42);
    assert_eq!(z80.pc(), 3);
    Ok(())
}

#[test]
fn ld_ix_displacement_writes_expected_cell() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x77, 0x05]);
    z80.set_ix(0x1000);
    z80.set_a(0x99);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(bus.read(0x1005), 0x99);
    Ok(())
}

#[test]
fn ld_ix_displacement_immediate_reads_the_value_after_the_displacement()
-> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x36, 0x05, 0x99]);
    z80.set_ix(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(bus.read(0x1005), 0x99);
    Ok(())
}

#[test]
fn displacement_is_signed() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x7E, 0xFB]);
    z80.set_ix(0x1000);
    bus.write(0x0FFB, 0x77);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(z80.a(), 0x77);
    Ok(())
}

#[test]
fn load_ix_immediate_and_memory_round_trip() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[
        0xDD, 0x21, 0x34, 0x12, // LD IX,0x1234
        0xDD, 0x22, 0x00, 0x20, // LD (0x2000),IX
        0xDD, 0x21, 0x00, 0x00, // LD IX,0
        0xDD, 0x2A, 0x00, 0x20, // LD IX,(0x2000)
    ]);
    assert_eq!(z80.step(&mut bus)?.t_states, 14);
    assert_eq!(z80.ix(), 0x1234);
    assert_eq!(z80.step(&mut bus)?.t_states, 20);
    assert_eq!(bus.read(0x2000), 0x34);
    assert_eq!(bus.read(0x2001), 0x12);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 20);
    assert_eq!(z80.ix(), 0x1234);
    Ok(())
}

#[test]
fn add_inc_and_dec_operate_on_the_index_pair() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x09, 0xDD, 0x23, 0xDD, 0x2B]);
    z80.set_ix(0x0FFF);
    z80.set_bc(0x0001);
    z80.set_f(FLAG_S | FLAG_Z);
    assert_eq!(z80.step(&mut bus)?.t_states, 15);
    assert_eq!(z80.ix(), 0x1000);
    assert_eq!(z80.f() & (FLAG_S | FLAG_Z), FLAG_S | FLAG_Z);
    assert_eq!(z80.f() & FLAG_H, FLAG_H);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.ix(), 0x1001);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.ix(), 0x1000);
    Ok(())
}

#[test]
fn push_pop_ex_and_stack_transfers_use_the_index_pair() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xE5, 0xDD, 0xE1]);
    z80.set_ix(0x1234);
    z80.set_sp(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 15);
    assert_eq!(z80.sp(), 0x0FFE);
    assert_eq!(bus.read(0x0FFE), 0x34);
    assert_eq!(bus.read(0x0FFF), 0x12);
    assert_eq!(z80.step(&mut bus)?.t_states, 14);
    assert_eq!(z80.ix(), 0x1234);
    assert_eq!(z80.sp(), 0x1000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xE3]);
    z80.set_ix(0xABCD);
    z80.set_sp(0x1000);
    bus.write(0x1000, 0x34);
    bus.write(0x1001, 0x12);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(z80.ix(), 0x1234);
    assert_eq!(bus.read(0x1000), 0xCD);
    assert_eq!(bus.read(0x1001), 0xAB);
    Ok(())
}

#[test]
fn jump_and_stack_pointer_use_the_index_pair() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xE9]);
    z80.set_ix(0x2000);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.pc(), 0x2000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xF9]);
    z80.set_ix(0x3000);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.sp(), 0x3000);
    Ok(())
}

#[test]
fn iy_prefix_uses_iy_for_displacement_and_cb() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xFD, 0x7E, 0x02, 0xFD, 0xCB, 0x03, 0xFE]);
    z80.set_iy(0x3000);
    bus.write(0x3002, 0x5A);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(z80.a(), 0x5A);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(bus.read(0x3003), 0x80);
    Ok(())
}

#[test]
fn cb_bit_with_ix_displacement_sets_z() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xCB, 0x05, 0x7E]);
    z80.set_ix(0x1000);
    bus.write(0x1005, 0x80);
    assert_eq!(z80.step(&mut bus)?.t_states, 20);
    assert_eq!(z80.f() & FLAG_Z, 0);
    assert_eq!(z80.f() & FLAG_S, FLAG_S);
    assert_eq!(z80.f() & FLAG_H, FLAG_H);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xCB, 0x05, 0x7E]);
    z80.set_ix(0x1000);
    z80.step(&mut bus)?;
    assert_eq!(z80.f() & FLAG_Z, FLAG_Z);
    Ok(())
}

#[test]
fn cb_set_res_and_shift_write_the_displaced_cell() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xCB, 0x05, 0xFE, 0xDD, 0xCB, 0x05, 0xBE]);
    z80.set_ix(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(bus.read(0x1005), 0x80);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(bus.read(0x1005), 0x00);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xCB, 0x05, 0x06]);
    z80.set_ix(0x1000);
    bus.write(0x1005, 0x81);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(bus.read(0x1005), 0x03);
    assert_eq!(z80.f() & FLAG_C, FLAG_C);
    Ok(())
}

#[test]
fn indexed_cb_copies_the_result_into_the_register_field() -> Result<(), Box<dyn Error>> {
    // `DD CB d 00` is the undocumented `RLC (IX+d),B` form: the memory cell and
    // the register field both receive the rotated value.
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xCB, 0x05, 0x00]);
    z80.set_ix(0x1000);
    bus.write(0x1005, 0x81);
    assert_eq!(z80.step(&mut bus)?.t_states, 23);
    assert_eq!(bus.read(0x1005), 0x03);
    assert_eq!(z80.bc() >> 8, 0x03);
    Ok(())
}

#[test]
fn ld_h_displacement_does_not_substitute_the_destination() -> Result<(), Box<dyn Error>> {
    // `DD 66 d` is `LD H,(IX+d)`: only the `(HL)` operand is replaced.
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x66, 0x05]);
    z80.set_ix(0x1000);
    bus.write(0x1005, 0x42);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(z80.hl() >> 8, 0x42);
    assert_eq!(z80.ix(), 0x1000);
    Ok(())
}

#[test]
fn repeated_prefix_is_a_bounded_four_t_state_step() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xDD, 0x00]);
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.pc(), 1);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.pc(), 3);
    Ok(())
}

#[test]
fn prefix_on_an_unrelated_opcode_is_a_wasted_fetch() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x00]);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.pc(), 2);
    Ok(())
}

#[test]
fn dd_ed_runs_the_ed_instruction() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xED, 0x56]);
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(z80.im(), 1);
    Ok(())
}

#[test]
fn dd_76_is_halt_and_dd_eb_is_ex_de_hl() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0x76]);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert!(z80.halted());

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDD, 0xEB]);
    z80.set_de(0x1234);
    z80.set_hl(0x5678);
    z80.set_ix(0x9ABC);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.de(), 0x5678);
    assert_eq!(z80.hl(), 0x1234);
    assert_eq!(z80.ix(), 0x9ABC);
    Ok(())
}
