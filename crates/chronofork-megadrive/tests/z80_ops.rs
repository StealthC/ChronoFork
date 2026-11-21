//! Main (unprefixed) Z80 opcode table: documented behaviour and T-states.

use std::error::Error;

use chronofork_megadrive::z80::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_Z, Z80, Z80Bus, Z80Error,
};
use proptest::collection::vec;
use proptest::prelude::*;

const RAM_SIZE: usize = 0x1_0000;
const DOCUMENTED_FLAGS: u8 = FLAG_S | FLAG_Z | FLAG_H | FLAG_PV | FLAG_N | FLAG_C;

/// 64 KiB RAM plus a record of I/O traffic.
#[derive(Debug)]
struct RamBus {
    ram: Vec<u8>,
    outputs: Vec<(u16, u8)>,
    input_value: u8,
}

impl RamBus {
    fn new(program: &[u8]) -> Self {
        let mut ram = vec![0_u8; RAM_SIZE];
        ram[..program.len()].copy_from_slice(program);
        Self {
            ram,
            outputs: Vec::new(),
            input_value: 0xFF,
        }
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
        Ok(self.input_value)
    }

    fn output(&mut self, port: u16, value: u8) -> Result<(), Z80Error> {
        self.outputs.push((port, value));
        Ok(())
    }
}

fn documented_flags(z80: &Z80) -> u8 {
    z80.f() & DOCUMENTED_FLAGS
}

#[test]
fn add_sets_documented_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x45, 0xC6, 0x38]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 7);
    assert_eq!(z80.a(), 0x7D);
    assert_eq!(documented_flags(&z80), 0);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x0F, 0xC6, 0x01]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x10);
    assert_eq!(documented_flags(&z80), FLAG_H);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0xFF, 0xC6, 0x01]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x00);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_H | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x7F, 0xC6, 0x01]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x80);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_H | FLAG_PV);
    Ok(())
}

#[test]
fn adc_uses_the_carry_flag() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x37, 0x3E, 0x00, 0xCE, 0x00]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 7);
    assert_eq!(z80.a(), 1);
    assert_eq!(documented_flags(&z80), 0);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x00, 0xCE, 0x00]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0);
    assert_eq!(documented_flags(&z80), FLAG_Z);
    Ok(())
}

#[test]
fn sub_and_cp_set_the_subtract_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x00, 0xD6, 0x01]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0xFF);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_H | FLAG_N | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x00, 0xFE, 0x01, 0xFE, 0x00]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0, "CP does not write the accumulator");
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_H | FLAG_N | FLAG_C);
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_N);
    Ok(())
}

#[test]
fn logical_operations_set_half_carry_parity_and_clear_carry() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0xF0, 0xE6, 0x0F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x00);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_H | FLAG_PV);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0xF0, 0xEE, 0xFF]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x0F);
    assert_eq!(documented_flags(&z80), FLAG_PV);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0xF0, 0xF6, 0x0F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0xFF);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_PV);
    Ok(())
}

#[test]
fn increment_and_decrement_preserve_carry() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x37, 0x3E, 0xFF, 0x3C]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.a(), 0x00);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_H | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x80, 0x3D]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x7F);
    assert_eq!(documented_flags(&z80), FLAG_H | FLAG_PV | FLAG_N);
    Ok(())
}

#[test]
fn add_pair_preserves_sign_and_zero() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_hl(0x0FFF);
    z80.set_bc(0x0001);
    z80.set_f(FLAG_S | FLAG_Z);
    let mut bus = RamBus::new(&[0x09]);
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(z80.hl(), 0x1000);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_Z | FLAG_H);
    Ok(())
}

#[test]
fn daa_adjusts_both_nibbles() -> Result<(), Box<dyn Error>> {
    // 0x45 + 0x38 = 0x7D, adjusted to the packed BCD sum 0x83.
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x45, 0xC6, 0x38, 0x27]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.a(), 0x83);
    assert_eq!(z80.f() & FLAG_C, 0);
    assert_eq!(z80.f() & FLAG_N, 0);
    assert_ne!(z80.f() & FLAG_H, 0);

    // 0x20 - 0x05 = 0x1B, adjusted to the packed BCD difference 0x15.
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x20, 0xD6, 0x05, 0x27]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x15);
    assert_ne!(z80.f() & FLAG_N, 0);
    assert_eq!(z80.f() & FLAG_C, 0);

    // 0x99 + 0x01 = 0x9A, adjusted to 0x00 with a decimal carry.
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x99, 0xC6, 0x01, 0x27]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x00);
    assert_ne!(z80.f() & FLAG_C, 0);
    assert_ne!(z80.f() & FLAG_Z, 0);
    Ok(())
}

#[test]
fn accumulator_rotates_preserve_sign_zero_and_parity() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x81, 0x07]);
    z80.step(&mut bus)?;
    z80.set_f(FLAG_S | FLAG_Z | FLAG_PV);
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x03);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_Z | FLAG_PV | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x01, 0x0F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x80);
    assert_eq!(documented_flags(&z80), FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x80, 0x17]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x00);
    assert_eq!(documented_flags(&z80), FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x01, 0x1F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x00);
    assert_eq!(documented_flags(&z80), FLAG_C);
    Ok(())
}

#[test]
fn cpl_scf_and_ccf_follow_the_documented_rules() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x55, 0x2F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0xAA);
    assert_eq!(documented_flags(&z80), FLAG_H | FLAG_N);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x37]);
    z80.step(&mut bus)?;
    assert_eq!(documented_flags(&z80), FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x37, 0x3F]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(documented_flags(&z80), FLAG_H);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3F]);
    z80.step(&mut bus)?;
    assert_eq!(documented_flags(&z80), FLAG_C);
    Ok(())
}

#[test]
fn load_instructions_cover_immediate_and_indirect_forms() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[
        0x06, 0x12, // LD B,0x12
        0x0E, 0x34, // LD C,0x34
        0x16, 0x56, // LD D,0x56
        0x1E, 0x78, // LD E,0x78
        0x26, 0x9A, // LD H,0x9A
        0x2E, 0xBC, // LD L,0xBC
        0x3E, 0xDE, // LD A,0xDE
        0x36, 0x42, // LD (HL),0x42
    ]);
    for _ in 0..8 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.bc(), 0x1234);
    assert_eq!(z80.de(), 0x5678);
    assert_eq!(z80.hl(), 0x9ABC);
    assert_eq!(z80.a(), 0xDE);
    assert_eq!(bus.read(0x9ABC), 0x42);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[
        0x21, 0x34, 0x12, // LD HL,0x1234
        0x22, 0x00, 0x20, // LD (0x2000),HL
        0x21, 0x00, 0x00, // LD HL,0
        0x2A, 0x00, 0x20, // LD HL,(0x2000)
    ]);
    for _ in 0..4 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.hl(), 0x1234);
    assert_eq!(bus.read(0x2000), 0x34);
    assert_eq!(bus.read(0x2001), 0x12);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[
        0x3E, 0x77, // LD A,0x77
        0x02, // LD (BC),A
        0x3E, 0x00, // LD A,0
        0x0A, // LD A,(BC)
        0x32, 0x00, 0x30, // LD (0x3000),A
        0x3E, 0x00, // LD A,0
        0x3A, 0x00, 0x30, // LD A,(0x3000)
    ]);
    z80.set_bc(0x2100);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(bus.read(0x2100), 0x77);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 7);
    assert_eq!(z80.a(), 0x77);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 13);
    assert_eq!(z80.a(), 0x77);
    assert_eq!(bus.read(0x3000), 0x77);
    Ok(())
}

#[test]
fn register_to_register_loads_copy_through_the_file() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_af(0x5A00);
    let mut bus = RamBus::new(&[0x47, 0x78]);
    // LD B,A; LD A,B
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x5A);
    z80.set_a(0x00);
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x5A);
    Ok(())
}

#[test]
fn increment_and_decrement_pair_cover_all_pairs() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x03, 0x13, 0x23, 0x33, 0x0B, 0x1B, 0x2B, 0x3B]);
    z80.set_bc(0xFFFF);
    z80.set_de(0x0001);
    z80.set_hl(0x00FF);
    z80.set_sp(0x0100);
    for _ in 0..4 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.bc(), 0x0000);
    assert_eq!(z80.de(), 0x0002);
    assert_eq!(z80.hl(), 0x0100);
    assert_eq!(z80.sp(), 0x0101);
    for _ in 0..4 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.bc(), 0xFFFF);
    assert_eq!(z80.de(), 0x0001);
    assert_eq!(z80.hl(), 0x00FF);
    assert_eq!(z80.sp(), 0x0100);
    Ok(())
}

#[test]
fn jumps_use_documented_relative_and_absolute_costs() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x18, 0x02, 0x00, 0x00, 0x3E, 0x01]);
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(z80.pc(), 4);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x28, 0x02]);
    z80.set_f(FLAG_Z);
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(z80.pc(), 4);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x28, 0x02]);
    assert_eq!(z80.step(&mut bus)?.t_states, 7);
    assert_eq!(z80.pc(), 2);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xC3, 0x00, 0x20]);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.pc(), 0x2000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xCA, 0x00, 0x20]);
    z80.set_f(FLAG_Z);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.pc(), 0x2000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xCA, 0x00, 0x20]);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.pc(), 3);
    Ok(())
}

#[test]
fn djnz_uses_taken_and_not_taken_costs() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x06, 0x02, 0x10, 0x00, 0x10, 0x00]);
    assert_eq!(z80.step(&mut bus)?.t_states, 7);
    assert_eq!(z80.step(&mut bus)?.t_states, 13);
    assert_eq!(z80.bc() >> 8, 1);
    assert_eq!(z80.pc(), 4);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.bc() >> 8, 0);
    assert_eq!(z80.pc(), 6);
    Ok(())
}

#[test]
fn calls_and_returns_use_the_stack_and_documented_costs() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xCD, 0x10, 0x00]);
    bus.write(0x10, 0xC9);
    assert_eq!(z80.step(&mut bus)?.t_states, 17);
    assert_eq!(z80.pc(), 0x10);
    assert_eq!(z80.sp(), 0xFFFE);
    assert_eq!(bus.read(0xFFFE), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x00);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.pc(), 3);
    assert_eq!(z80.sp(), 0x0000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xCC, 0x10, 0x00]);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.pc(), 3);
    assert_eq!(z80.sp(), 0);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xC8]);
    bus.write(0x1000, 0x34);
    bus.write(0x1001, 0x12);
    z80.set_sp(0x1000);
    z80.set_f(FLAG_Z);
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(z80.pc(), 0x1234);
    assert_eq!(z80.sp(), 0x1002);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xC8]);
    z80.set_sp(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 5);
    assert_eq!(z80.sp(), 0x1000);
    Ok(())
}

#[test]
fn push_pop_restart_and_exchange_stack_round_trip() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    z80.set_bc(0x1234);
    z80.set_sp(0x1000);
    let mut bus = RamBus::new(&[0xC5, 0xD1]);
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(z80.sp(), 0x0FFE);
    assert_eq!(z80.step(&mut bus)?.t_states, 10);
    assert_eq!(z80.de(), 0x1234);
    assert_eq!(z80.sp(), 0x1000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xFF]);
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(z80.pc(), 0x0038);
    assert_eq!(z80.sp(), 0xFFFE);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xE3]);
    z80.set_hl(0xABCD);
    z80.set_sp(0x1000);
    bus.write(0x1000, 0x34);
    bus.write(0x1001, 0x12);
    assert_eq!(z80.step(&mut bus)?.t_states, 19);
    assert_eq!(z80.hl(), 0x1234);
    assert_eq!(bus.read(0x1000), 0xCD);
    assert_eq!(bus.read(0x1001), 0xAB);
    Ok(())
}

#[test]
fn exchange_instructions_swap_the_documented_sets() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x08, 0x08]);
    z80.set_af(0x1122);
    z80.step(&mut bus)?;
    assert_eq!(z80.af(), 0);
    z80.set_af(0x3344);
    z80.step(&mut bus)?;
    assert_eq!(z80.af(), 0x1122);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xD9]);
    z80.set_af(0xABCD);
    z80.set_bc(0x1122);
    z80.set_de(0x3344);
    z80.set_hl(0x5566);
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.af(), 0xABCD, "EXX swaps the general registers only");
    assert_eq!(z80.bc(), 0);
    assert_eq!(z80.de(), 0);
    assert_eq!(z80.hl(), 0);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xEB]);
    z80.set_de(0x1234);
    z80.set_hl(0x5678);
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.de(), 0x5678);
    assert_eq!(z80.hl(), 0x1234);
    Ok(())
}

#[test]
fn jump_pair_and_load_stack_pointer_forms() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xE9]);
    z80.set_hl(0x2000);
    assert_eq!(z80.step(&mut bus)?.t_states, 4);
    assert_eq!(z80.pc(), 0x2000);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xF9]);
    z80.set_hl(0x3000);
    assert_eq!(z80.step(&mut bus)?.t_states, 6);
    assert_eq!(z80.sp(), 0x3000);
    Ok(())
}

#[test]
fn immediate_io_uses_the_accumulator_as_the_high_port_byte() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0x3E, 0x12, 0xD3, 0x34]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(bus.outputs, vec![(0x1234, 0x12)]);

    let mut z80 = Z80::new();
    let mut bus = RamBus::new(&[0xDB, 0x34]);
    bus.input_value = 0x5A;
    z80.set_f(FLAG_S | FLAG_Z);
    assert_eq!(z80.step(&mut bus)?.t_states, 11);
    assert_eq!(z80.a(), 0x5A);
    assert_eq!(z80.f(), FLAG_S | FLAG_Z, "IN A,(n) does not affect flags");
    Ok(())
}

struct MirrorBus {
    bytes: Vec<u8>,
}

impl Z80Bus for MirrorBus {
    fn read8(&mut self, address: u16) -> Result<u8, Z80Error> {
        if self.bytes.is_empty() {
            return Ok(0);
        }
        Ok(self.bytes[usize::from(address) % self.bytes.len()])
    }

    fn write8(&mut self, _address: u16, _value: u8) -> Result<(), Z80Error> {
        Ok(())
    }

    fn input(&mut self, _port: u16) -> Result<u8, Z80Error> {
        Ok(0)
    }

    fn output(&mut self, _port: u16, _value: u8) -> Result<(), Z80Error> {
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Arbitrary bounded instruction streams must either decode or report a
    /// typed error; no input may panic or execute an unbounded number of
    /// T-states in one step.
    #[test]
    fn arbitrary_instruction_stream_never_panics(bytes in vec(any::<u8>(), 0..256)) {
        let mut z80 = Z80::new();
        let mut bus = MirrorBus { bytes };
        for _ in 0..64 {
            match z80.step(&mut bus) {
                Ok(outcome) => {
                    prop_assert!(outcome.t_states <= 64, "cost {} out of range", outcome.t_states);
                    prop_assert!(outcome.master_cycles() % 15 == 0);
                }
                Err(error) => prop_assert!(!error.to_string().is_empty()),
            }
            prop_assert!(z80.im() <= 2);
        }
    }
}
