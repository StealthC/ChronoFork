//! Effective-address decode and bus access for the 68000 core.
//!
//! The 68000 encodes an effective address in six bits: a three-bit mode and a
//! three-bit register. [`decode_ea`] turns that pair into an [`Ea`] value
//! without touching the bus; [`read_ea`], [`write_ea`] and [`ea_address`] then
//! fetch the extension words in instruction order and perform the access.
//!
//! Extension words belong to the instruction stream, so every helper here uses
//! the CPU's program counter and advances it exactly as the instruction does:
//! displacement and index words are consumed in encoding order, and the
//! PC-relative base is the address of the extension word itself.

use crate::bus::MegadriveBus;

use super::{CpuError, M68000};

/// Operand width of a 68000 instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    /// One byte.
    Byte,
    /// One word (two bytes).
    Word,
    /// One long word (four bytes).
    Long,
}

impl Size {
    /// Returns the operand width in bytes.
    #[must_use]
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 4,
        }
    }

    /// Returns the mask that keeps exactly the operand bits.
    #[must_use]
    pub const fn mask(self) -> u32 {
        match self {
            Self::Byte => 0x0000_00FF,
            Self::Word => 0x0000_FFFF,
            Self::Long => 0xFFFF_FFFF,
        }
    }

    /// Returns the mask of the operand's sign bit.
    pub(crate) const fn sign_bit(self) -> u32 {
        match self {
            Self::Byte => 0x0000_0080,
            Self::Word => 0x0000_8000,
            Self::Long => 0x8000_0000,
        }
    }

    /// Sign-extends the low `self` bits of `value` to 32 bits.
    #[must_use]
    pub const fn sign_extend(self, value: u32) -> u32 {
        match self {
            Self::Byte => (value as u8 as i8) as i32 as u32,
            Self::Word => (value as u16 as i16) as i32 as u32,
            Self::Long => value,
        }
    }
}

/// A decoded 68000 effective address.
///
/// The register index is always 0..=7. `Abs16`/`Abs32`, the two PC-relative
/// forms and `Imm` carry no register because the extension word supplies the
/// address or value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ea {
    /// Data register direct, `Dn`.
    Dn(u8),
    /// Address register direct, `An`.
    An(u8),
    /// Address register indirect, `(An)`.
    Indirect(u8),
    /// Address register indirect with post-increment, `(An)+`.
    PostInc(u8),
    /// Address register indirect with pre-decrement, `-(An)`.
    PreDec(u8),
    /// Address register indirect with a 16-bit displacement, `(d16,An)`.
    Disp16(u8),
    /// Address register indirect with an 8-bit displacement and index,
    /// `(d8,An,Xn)`.
    Index(u8),
    /// Absolute short address, `(xxx).W`.
    Abs16,
    /// Absolute long address, `(xxx).L`.
    Abs32,
    /// Program counter with a 16-bit displacement, `(d16,PC)`.
    PcDisp16,
    /// Program counter with an 8-bit displacement and index, `(d8,PC,Xn)`.
    PcIndex,
    /// Immediate operand.
    Imm,
}

/// Decodes the mode/register pair into an effective address.
///
/// Byte-sized address-register direct operands are rejected here: the 68000
/// treats a byte access to `An` as an address error, not an instruction.
pub fn decode_ea(mode: u8, register: u8, size: Size) -> Result<Ea, CpuError> {
    if mode > 7 || register > 7 {
        return Err(CpuError::Unsupported(
            "effective-address mode or register is out of range".to_owned(),
        ));
    }
    let ea = match mode {
        0 => Ea::Dn(register),
        1 => {
            if size == Size::Byte {
                return Err(CpuError::Unsupported(
                    "byte-sized address-register operand".to_owned(),
                ));
            }
            Ea::An(register)
        }
        2 => Ea::Indirect(register),
        3 => Ea::PostInc(register),
        4 => Ea::PreDec(register),
        5 => Ea::Disp16(register),
        6 => Ea::Index(register),
        _ => match register {
            0 => Ea::Abs16,
            1 => Ea::Abs32,
            2 => Ea::PcDisp16,
            3 => Ea::PcIndex,
            4 => Ea::Imm,
            other => {
                return Err(CpuError::Unsupported(format!(
                    "reserved effective-address encoding 7/{other}"
                )));
            }
        },
    };
    Ok(ea)
}

/// Returns the address an effective address denotes without reading it.
///
/// This is the address-only access used by `LEA`, `PEA` and `CHK`: only the
/// control modes are valid, so a data-register, address-register, immediate or
/// auto-increment/decrement form is a typed error. Extension words are consumed
/// from the instruction stream.
pub fn ea_address(cpu: &mut M68000, bus: &mut MegadriveBus, ea: Ea) -> Result<u32, CpuError> {
    match ea {
        Ea::Indirect(register) => Ok(cpu.read_an(usize::from(register))),
        Ea::Disp16(register) => {
            let displacement = i32::from(cpu.fetch_u16(bus)? as i16);
            add_signed(cpu.read_an(usize::from(register)), displacement)
        }
        Ea::Index(register) => {
            let extension = cpu.fetch_u16(bus)?;
            let index = index_value(cpu, extension);
            let displacement = i32::from(extension as i8);
            let base = add_signed(cpu.read_an(usize::from(register)), displacement)?;
            add_signed(base, index as i32)
        }
        Ea::Abs16 => Ok(Size::Word.sign_extend(u32::from(cpu.fetch_u16(bus)?))),
        Ea::Abs32 => cpu.fetch_u32(bus),
        Ea::PcDisp16 => {
            let base = cpu.pc();
            let displacement = i32::from(cpu.fetch_u16(bus)? as i16);
            add_signed(base, displacement)
        }
        Ea::PcIndex => {
            let base = cpu.pc();
            let extension = cpu.fetch_u16(bus)?;
            let index = index_value(cpu, extension);
            let displacement = i32::from(extension as i8);
            let address = add_signed(base, displacement)?;
            add_signed(address, index as i32)
        }
        Ea::Dn(_) | Ea::An(_) | Ea::PostInc(_) | Ea::PreDec(_) | Ea::Imm => Err(
            CpuError::Unsupported("effective address is not a control addressing mode".to_owned()),
        ),
    }
}

/// Reads the operand denoted by `ea` at `size`.
///
/// Auto-increment and pre-decrement forms update the address register only
/// after a successful bus access, and the increment is the operand size in
/// bytes (one, two or four).
pub fn read_ea(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    ea: Ea,
    size: Size,
) -> Result<u32, CpuError> {
    match ea {
        Ea::Dn(register) => Ok(cpu.read_dn(usize::from(register), size)),
        Ea::An(register) => read_an(cpu, register, size),
        Ea::Indirect(register) => read_memory(bus, cpu.read_an(usize::from(register)), size),
        Ea::PostInc(register) => {
            let address = cpu.read_an(usize::from(register));
            let value = read_memory(bus, address, size)?;
            cpu.write_an(usize::from(register), advance(address, size.bytes())?);
            Ok(value)
        }
        Ea::PreDec(register) => {
            let address = retreat(cpu.read_an(usize::from(register)), size.bytes())?;
            let value = read_memory(bus, address, size)?;
            cpu.write_an(usize::from(register), address);
            Ok(value)
        }
        Ea::Disp16(_) | Ea::Index(_) | Ea::Abs16 | Ea::Abs32 | Ea::PcDisp16 | Ea::PcIndex => {
            let address = ea_address(cpu, bus, ea)?;
            read_memory(bus, address, size)
        }
        Ea::Imm => match size {
            Size::Byte | Size::Word => Ok(u32::from(cpu.fetch_u16(bus)?) & size.mask()),
            Size::Long => cpu.fetch_u32(bus),
        },
    }
}

/// Writes `value` to the operand denoted by `ea` at `size`.
///
/// The 68000 treats a byte-sized write to `An` as an address error. For memory
/// destinations the whole range is validated by the bus before any byte is
/// written, so a faulting access never half-updates memory; the address
/// register of an auto-increment/decrement form is left unchanged on failure.
pub fn write_ea(
    cpu: &mut M68000,
    bus: &mut MegadriveBus,
    ea: Ea,
    size: Size,
    value: u32,
) -> Result<(), CpuError> {
    match ea {
        Ea::Dn(register) => {
            cpu.write_dn(usize::from(register), value, size);
            Ok(())
        }
        Ea::An(register) => write_an(cpu, register, size, value),
        Ea::Indirect(register) => {
            write_memory(bus, cpu.read_an(usize::from(register)), size, value)
        }
        Ea::PostInc(register) => {
            let address = cpu.read_an(usize::from(register));
            write_memory(bus, address, size, value)?;
            cpu.write_an(usize::from(register), advance(address, size.bytes())?);
            Ok(())
        }
        Ea::PreDec(register) => {
            let address = retreat(cpu.read_an(usize::from(register)), size.bytes())?;
            write_memory(bus, address, size, value)?;
            cpu.write_an(usize::from(register), address);
            Ok(())
        }
        Ea::Disp16(_) | Ea::Index(_) | Ea::Abs16 | Ea::Abs32 | Ea::PcDisp16 | Ea::PcIndex => {
            let address = ea_address(cpu, bus, ea)?;
            write_memory(bus, address, size, value)
        }
        Ea::Imm => Err(CpuError::Unsupported(
            "immediate operand cannot be a destination".to_owned(),
        )),
    }
}

fn read_an(cpu: &M68000, register: u8, size: Size) -> Result<u32, CpuError> {
    let value = cpu.read_an(usize::from(register));
    if size == Size::Byte {
        return Err(CpuError::AddressError { address: value });
    }
    Ok(value & size.mask())
}

fn write_an(cpu: &mut M68000, register: u8, size: Size, value: u32) -> Result<(), CpuError> {
    if size == Size::Byte {
        return Err(CpuError::AddressError {
            address: cpu.read_an(usize::from(register)),
        });
    }
    cpu.write_an(usize::from(register), size.sign_extend(value));
    Ok(())
}

fn read_memory(bus: &mut MegadriveBus, address: u32, size: Size) -> Result<u32, CpuError> {
    Ok(match size {
        Size::Byte => u32::from(bus.read8(address)?),
        Size::Word => u32::from(bus.read16(address)?),
        Size::Long => bus.read32(address)?,
    })
}

fn write_memory(
    bus: &mut MegadriveBus,
    address: u32,
    size: Size,
    value: u32,
) -> Result<(), CpuError> {
    match size {
        Size::Byte => bus.write8(address, value as u8)?,
        Size::Word => bus.write16(address, value as u16)?,
        Size::Long => bus.write32(address, value)?,
    }
    Ok(())
}

fn advance(address: u32, amount: u32) -> Result<u32, CpuError> {
    address.checked_add(amount).ok_or(CpuError::AddressOverflow)
}

fn retreat(address: u32, amount: u32) -> Result<u32, CpuError> {
    address.checked_sub(amount).ok_or(CpuError::StackOverflow)
}

fn add_signed(base: u32, displacement: i32) -> Result<u32, CpuError> {
    let target = i64::from(base) + i64::from(displacement);
    if target < 0 || target > i64::from(u32::MAX) {
        return Err(CpuError::AddressOverflow);
    }
    // Proven by the range check above.
    Ok(target as u32)
}

/// Resolves the index register and sign-extends it according to its size bit.
fn index_value(cpu: &M68000, extension: u16) -> u32 {
    let register = usize::from((extension >> 12) & 0x7);
    let value = if extension & 0x8000 != 0 {
        cpu.read_an(register)
    } else {
        cpu.read_dn(register, Size::Long)
    };
    if extension & 0x0800 != 0 {
        value
    } else {
        (value as u16 as i16) as i32 as u32
    }
}
