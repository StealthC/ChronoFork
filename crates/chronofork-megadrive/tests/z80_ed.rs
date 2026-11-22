//! `ED`-prefixed Z80 instructions.

use std::error::Error;

use chronofork_megadrive::z80::{
    FLAG_C, FLAG_H, FLAG_N, FLAG_PV, FLAG_S, FLAG_Z, Z80, Z80Bus, Z80Error,
};

const RAM_SIZE: usize = 0x1_0000;
const DOCUMENTED_FLAGS: u8 = FLAG_S | FLAG_Z | FLAG_H | FLAG_PV | FLAG_N | FLAG_C;

#[derive(Debug)]
struct IoBus {
    ram: Vec<u8>,
    outputs: Vec<(u16, u8)>,
    input_value: u8,
}

impl IoBus {
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

impl Z80Bus for IoBus {
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
fn ldir_copies_block_and_stops() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[
        0x21, 0x00, 0x10, // LD HL,0x1000
        0x11, 0x00, 0x20, // LD DE,0x2000
        0x01, 0x03, 0x00, // LD BC,3
        0xED, 0xB0, // LDIR
    ]);
    for (offset, value) in [0x01_u8, 0x02, 0x03].iter().enumerate() {
        let address = u16::try_from(0x1000 + offset)?;
        bus.write(address, *value);
    }
    for _ in 0..3 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.read(0x2000), 0x01);
    assert_eq!(bus.read(0x2001), 0x02);
    assert_eq!(bus.read(0x2002), 0x03);
    assert_eq!(z80.hl(), 0x1003);
    assert_eq!(z80.de(), 0x2003);
    assert_eq!(z80.bc(), 0);
    assert_eq!(z80.f() & FLAG_PV, 0);
    Ok(())
}

#[test]
fn lddr_copies_downwards() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[
        0x21, 0x02, 0x10, // LD HL,0x1002
        0x11, 0x02, 0x20, // LD DE,0x2002
        0x01, 0x02, 0x00, // LD BC,2
        0xED, 0xB8, // LDDR
    ]);
    bus.write(0x1001, 0xAA);
    bus.write(0x1002, 0xBB);
    for _ in 0..3 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.read(0x2001), 0xAA);
    assert_eq!(bus.read(0x2002), 0xBB);
    assert_eq!(z80.hl(), 0x1000);
    assert_eq!(z80.de(), 0x2000);
    assert_eq!(z80.bc(), 0);
    Ok(())
}

#[test]
fn cpir_stops_at_the_matching_byte() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[
        0x3E, 0x05, // LD A,5
        0x21, 0x00, 0x10, // LD HL,0x1000
        0x01, 0x04, 0x00, // LD BC,4
        0xED, 0xB1, // CPIR
    ]);
    for (offset, value) in [0x01_u8, 0x02, 0x05, 0x04].iter().enumerate() {
        let address = u16::try_from(0x1000 + offset)?;
        bus.write(address, *value);
    }
    for _ in 0..3 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(z80.hl(), 0x1003);
    assert_eq!(z80.bc(), 1);
    assert_eq!(z80.f() & FLAG_Z, FLAG_Z);
    assert_eq!(z80.f() & FLAG_N, FLAG_N);
    Ok(())
}

#[test]
fn interrupt_modes_are_latched() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x46, 0xED, 0x56, 0xED, 0x5E]);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.im(), 0);
    z80.step(&mut bus)?;
    assert_eq!(z80.im(), 1);
    z80.step(&mut bus)?;
    assert_eq!(z80.im(), 2);
    // Accepting an interrupt through the vector table is a later 8.4 task.
    Ok(())
}

#[test]
fn rrd_rld_nibble_transfer() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x21, 0x00, 0x10, 0x3E, 0x12, 0xED, 0x67]);
    bus.write(0x1000, 0x34);
    for _ in 0..2 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 18);
    assert_eq!(z80.a(), 0x14);
    assert_eq!(bus.read(0x1000), 0x23);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x21, 0x00, 0x10, 0x3E, 0x12, 0xED, 0x6F]);
    bus.write(0x1000, 0x34);
    for _ in 0..2 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 18);
    assert_eq!(z80.a(), 0x13);
    assert_eq!(bus.read(0x1000), 0x42);
    Ok(())
}

#[test]
fn neg_sets_the_documented_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x3E, 0x01, 0xED, 0x44]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.a(), 0xFF);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_H | FLAG_N | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x3E, 0x00, 0xED, 0x44]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_N);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x3E, 0x80, 0xED, 0x44]);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x80);
    assert_eq!(documented_flags(&z80), FLAG_S | FLAG_PV | FLAG_N | FLAG_C);
    Ok(())
}

#[test]
fn retn_and_reti_restore_iff1_from_iff2() -> Result<(), Box<dyn Error>> {
    for opcode in [0x45_u8, 0x4D] {
        let mut z80 = Z80::new();
        let mut bus = IoBus::new(&[0xED, opcode]);
        bus.write(0x2000, 0x34);
        bus.write(0x2001, 0x12);
        z80.set_sp(0x2000);
        z80.set_iff1(false);
        z80.set_iff2(true);
        assert_eq!(z80.step(&mut bus)?.t_states, 14, "opcode {opcode:#04x}");
        assert_eq!(z80.pc(), 0x1234);
        assert_eq!(z80.sp(), 0x2002);
        assert!(z80.iff1());
    }
    Ok(())
}

#[test]
fn ld_a_i_and_ld_a_r_copy_iff2_into_parity_overflow() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x57]);
    z80.set_i(0x55);
    z80.set_iff2(true);
    z80.set_f(FLAG_C);
    assert_eq!(z80.step(&mut bus)?.t_states, 9);
    assert_eq!(z80.a(), 0x55);
    assert_eq!(documented_flags(&z80), FLAG_PV | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x5F]);
    z80.set_r(0x5A);
    assert_eq!(z80.step(&mut bus)?.t_states, 9);
    // The two M1 fetches (`ED` and the opcode) advance R before execution.
    assert_eq!(z80.a(), 0x5C);
    assert_eq!(z80.f() & FLAG_PV, 0);
    Ok(())
}

#[test]
fn ld_i_a_and_ld_r_a_write_the_special_registers() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x3E, 0x42, 0xED, 0x47, 0xED, 0x4F]);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 9);
    assert_eq!(z80.i(), 0x42);
    assert_eq!(z80.step(&mut bus)?.t_states, 9);
    assert_eq!(z80.r(), 0x42);
    Ok(())
}

#[test]
fn input_forms_set_flags_and_preserve_carry() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x78]);
    bus.input_value = 0x5A;
    z80.set_bc(0x1234);
    z80.set_f(FLAG_C);
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(z80.a(), 0x5A);
    assert_eq!(documented_flags(&z80), FLAG_PV | FLAG_C);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x40, 0xED, 0x70]);
    bus.input_value = 0x01;
    z80.set_bc(0x5600);
    z80.step(&mut bus)?;
    assert_eq!(z80.bc() >> 8, 0x01);
    z80.set_a(0x77);
    z80.step(&mut bus)?;
    assert_eq!(z80.a(), 0x77, "IN (C) only updates flags");
    assert_eq!(z80.f() & FLAG_PV, 0);
    Ok(())
}

#[test]
fn output_forms_send_the_documented_port_and_value() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x3E, 0x5A, 0xED, 0x79]);
    z80.set_bc(0x1234);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 12);
    assert_eq!(bus.outputs, vec![(0x1234, 0x5A)]);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x71]);
    z80.set_bc(0x1234);
    z80.step(&mut bus)?;
    assert_eq!(bus.outputs, vec![(0x1234, 0x00)]);
    Ok(())
}

#[test]
fn ldi_and_cpi_set_their_documented_flags() -> Result<(), Box<dyn Error>> {
    // LDI advances both pointers, decrements BC and reports BC != 0 in PV.
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xA0]);
    bus.write(0x1000, 0x5A);
    z80.set_hl(0x1000);
    z80.set_de(0x2000);
    z80.set_bc(0x0002);
    z80.set_f(FLAG_C);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.read(0x2000), 0x5A);
    assert_eq!(z80.hl(), 0x1001);
    assert_eq!(z80.de(), 0x2001);
    assert_eq!(z80.bc(), 1);
    assert_eq!(documented_flags(&z80), FLAG_PV | FLAG_C);

    // CPI compares A with (HL) without writing A; H, Z and N follow the
    // subtraction and PV reports BC != 0 after the decrement.
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xA1]);
    bus.write(0x1000, 0x05);
    z80.set_hl(0x1000);
    z80.set_bc(0x0001);
    z80.set_af(0x0500);
    z80.set_f(FLAG_C);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(z80.a(), 0x05);
    assert_eq!(z80.hl(), 0x1001);
    assert_eq!(z80.bc(), 0);
    assert_eq!(documented_flags(&z80), FLAG_Z | FLAG_N | FLAG_C);
    Ok(())
}

#[test]
fn cpdr_searches_downwards_and_stops_at_the_match() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[
        0x3E, 0x05, // LD A,5
        0x21, 0x02, 0x10, // LD HL,0x1002
        0x01, 0x03, 0x00, // LD BC,3
        0xED, 0xB9, // CPDR
    ]);
    bus.write(0x1000, 0x05);
    bus.write(0x1001, 0x09);
    bus.write(0x1002, 0x09);
    for _ in 0..3 {
        z80.step(&mut bus)?;
    }
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(z80.hl(), 0x0FFF);
    assert_eq!(z80.bc(), 0);
    assert_eq!(z80.f() & FLAG_Z, FLAG_Z);
    Ok(())
}

#[test]
fn repeating_block_output_sends_each_byte() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xB3]);
    bus.write(0x1000, 0x11);
    bus.write(0x1001, 0x22);
    z80.set_bc(0x0255);
    z80.set_hl(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.outputs, vec![(0x0155, 0x11), (0x0055, 0x22)]);
    assert_eq!(z80.bc() >> 8, 0);
    assert_eq!(z80.hl(), 0x1002);
    Ok(())
}

#[test]
fn block_input_forms_update_memory_and_the_b_counter() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xA2]);
    bus.input_value = 0xAA;
    z80.set_bc(0x0255);
    z80.set_hl(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.read(0x1000), 0xAA);
    assert_eq!(z80.bc() >> 8, 0x01);
    assert_eq!(z80.hl(), 0x1001);
    assert_eq!(z80.f() & FLAG_Z, 0);
    assert_eq!(z80.f() & FLAG_N, FLAG_N);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xB2]);
    bus.input_value = 0xAA;
    z80.set_bc(0x0255);
    z80.set_hl(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 21);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.read(0x1000), 0xAA);
    assert_eq!(bus.read(0x1001), 0xAA);
    assert_eq!(z80.bc() >> 8, 0);
    assert_eq!(z80.f() & FLAG_Z, FLAG_Z);
    Ok(())
}

#[test]
fn block_output_forms_send_memory_bytes() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0xA3]);
    bus.write(0x1000, 0x80);
    z80.set_bc(0x0255);
    z80.set_hl(0x1000);
    assert_eq!(z80.step(&mut bus)?.t_states, 16);
    assert_eq!(bus.outputs, vec![(0x0155, 0x80)]);
    assert_eq!(z80.bc() >> 8, 0x01);
    assert_eq!(z80.hl(), 0x1001);
    assert_eq!(z80.f() & FLAG_N, FLAG_N);
    Ok(())
}

#[test]
fn adc_and_sbc_hl_use_sixteen_bit_flags() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x37, 0xED, 0x4A]);
    z80.set_hl(0x0FFF);
    z80.set_bc(0x0001);
    z80.step(&mut bus)?;
    assert_eq!(z80.step(&mut bus)?.t_states, 15);
    assert_eq!(z80.hl(), 0x1001);
    assert_eq!(documented_flags(&z80), FLAG_H);

    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0x37, 0xED, 0x42]);
    z80.set_hl(0x1001);
    z80.set_bc(0x0001);
    z80.step(&mut bus)?;
    z80.step(&mut bus)?;
    assert_eq!(z80.hl(), 0x0FFF);
    assert_eq!(documented_flags(&z80), FLAG_H | FLAG_N);
    Ok(())
}

#[test]
fn extended_load_forms_move_pairs_to_and_from_memory() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x43, 0x00, 0x20, 0xED, 0x4B, 0x00, 0x20]);
    z80.set_bc(0x1234);
    assert_eq!(z80.step(&mut bus)?.t_states, 20);
    assert_eq!(bus.read(0x2000), 0x34);
    assert_eq!(bus.read(0x2001), 0x12);
    z80.set_bc(0);
    assert_eq!(z80.step(&mut bus)?.t_states, 20);
    assert_eq!(z80.bc(), 0x1234);
    Ok(())
}

#[test]
fn undefined_ed_opcodes_are_two_byte_nops() -> Result<(), Box<dyn Error>> {
    let mut z80 = Z80::new();
    let mut bus = IoBus::new(&[0xED, 0x00, 0xED, 0xBC]);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.pc(), 2);
    assert_eq!(z80.r(), 2);
    assert_eq!(z80.af(), 0);
    assert_eq!(z80.bc(), 0);
    assert_eq!(z80.de(), 0);
    assert_eq!(z80.hl(), 0);
    assert_eq!(z80.step(&mut bus)?.t_states, 8);
    assert_eq!(z80.pc(), 4);
    Ok(())
}
