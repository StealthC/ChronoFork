//! Big-endian Mega Drive memory bus for the first backend slice.
//!
//! Only the ROM cartridge window and the 64 KiB work RAM are mapped. VDP, PSG,
//! Z80, controller and mapper windows are deliberately absent from this slice;
//! accessing them is an explicit error rather than silent open-bus behavior.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// First address of the cartridge ROM window.
pub const ROM_BASE: u32 = 0x0000_0000;
/// Exclusive end of the cartridge ROM window on the 68000 bus.
pub const ROM_END: u32 = 0x0040_0000;
/// First address of the 64 KiB work RAM window.
pub const RAM_BASE: u32 = 0x00FF_0000;
/// Exclusive end of the work RAM window.
pub const RAM_END: u32 = 0x0100_0000;
/// Size of work RAM in bytes.
pub const RAM_SIZE: usize = 0x0001_0000;

/// A bus access that the current slice cannot satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BusError {
    /// A read outside ROM or RAM.
    UnmappedRead {
        /// Requested address.
        address: u32,
    },
    /// A write outside ROM or RAM.
    UnmappedWrite {
        /// Requested address.
        address: u32,
    },
    /// A write to the read-only cartridge window.
    WriteProtected {
        /// Requested address.
        address: u32,
    },
    /// A word or long access at an odd address.
    OddAddress {
        /// Requested address.
        address: u32,
    },
    /// Address arithmetic overflowed.
    AddressOverflow {
        /// Starting address.
        address: u32,
        /// Requested byte count.
        length: usize,
    },
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmappedRead { address } => {
                write!(f, "unmapped read at 0x{address:06X}")
            }
            Self::UnmappedWrite { address } => {
                write!(f, "unmapped write at 0x{address:06X}")
            }
            Self::WriteProtected { address } => {
                write!(f, "write to cartridge ROM at 0x{address:06X}")
            }
            Self::OddAddress { address } => {
                write!(f, "odd-address access at 0x{address:06X}")
            }
            Self::AddressOverflow { address, length } => write!(
                f,
                "address overflow at 0x{address:06X} for {length} byte(s)"
            ),
        }
    }
}

impl Error for BusError {}

/// ROM + 64 KiB work RAM, accessed big-endian like the 68000 bus.
#[derive(Clone, Debug)]
pub struct MegadriveBus {
    rom: Arc<[u8]>,
    ram: Vec<u8>,
}

impl MegadriveBus {
    /// Creates a bus over an immutable cartridge image.
    ///
    /// The factory validates the image size before constructing a bus.
    #[must_use]
    pub fn new(rom: Arc<[u8]>) -> Self {
        Self {
            rom,
            ram: vec![0; RAM_SIZE],
        }
    }

    /// Returns the cartridge image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Returns the work RAM.
    #[must_use]
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Reads one byte.
    pub fn read8(&self, address: u32) -> Result<u8, BusError> {
        if address < ROM_END {
            let index = usize::try_from(address).map_err(|_| BusError::UnmappedRead { address })?;
            return self
                .rom
                .get(index)
                .copied()
                .ok_or(BusError::UnmappedRead { address });
        }
        if (RAM_BASE..RAM_END).contains(&address) {
            let offset = usize::try_from(address & 0xFFFF)
                .map_err(|_| BusError::UnmappedRead { address })?;
            return self
                .ram
                .get(offset)
                .copied()
                .ok_or(BusError::UnmappedRead { address });
        }
        Err(BusError::UnmappedRead { address })
    }

    /// Reads a big-endian word.
    pub fn read16(&self, address: u32) -> Result<u16, BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        let high = self.read8(address)?;
        let low = self.read8(
            address
                .checked_add(1)
                .ok_or(BusError::AddressOverflow { address, length: 2 })?,
        )?;
        Ok(u16::from_be_bytes([high, low]))
    }

    /// Reads a big-endian long word.
    pub fn read32(&self, address: u32) -> Result<u32, BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        let high = u32::from(self.read16(address)?);
        let low = u32::from(
            self.read16(
                address
                    .checked_add(2)
                    .ok_or(BusError::AddressOverflow { address, length: 4 })?,
            )?,
        );
        Ok((high << 16) | low)
    }

    /// Validates that a write range lies entirely in work RAM.
    ///
    /// Validation happens before any byte is mutated so an invalid range cannot
    /// leave a partially written machine state.
    fn ensure_ram_write_range(&self, address: u32, length: usize) -> Result<(), BusError> {
        if length == 0 {
            return Ok(());
        }
        let span = u32::try_from(length.saturating_sub(1))
            .map_err(|_| BusError::UnmappedWrite { address })?;
        let end = address
            .checked_add(span)
            .ok_or(BusError::AddressOverflow { address, length })?;
        if address < RAM_BASE || end >= RAM_END {
            if address < ROM_END {
                return Err(BusError::WriteProtected { address });
            }
            return Err(BusError::UnmappedWrite { address: end });
        }
        Ok(())
    }

    /// Writes one byte to work RAM.
    pub fn write8(&mut self, address: u32, value: u8) -> Result<(), BusError> {
        self.ensure_ram_write_range(address, 1)?;
        let offset =
            usize::try_from(address & 0xFFFF).map_err(|_| BusError::UnmappedWrite { address })?;
        if let Some(slot) = self.ram.get_mut(offset) {
            *slot = value;
            return Ok(());
        }
        Err(BusError::UnmappedWrite { address })
    }

    /// Writes a big-endian word to work RAM.
    pub fn write16(&mut self, address: u32, value: u16) -> Result<(), BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        self.ensure_ram_write_range(address, 2)?;
        let bytes = value.to_be_bytes();
        self.write8(address, bytes[0])?;
        let second = address
            .checked_add(1)
            .ok_or(BusError::AddressOverflow { address, length: 2 })?;
        self.write8(second, bytes[1])
    }

    /// Writes a big-endian long word to work RAM.
    pub fn write32(&mut self, address: u32, value: u32) -> Result<(), BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        self.ensure_ram_write_range(address, 4)?;
        let bytes = value.to_be_bytes();
        for (offset, byte) in bytes.iter().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| BusError::UnmappedWrite { address })?;
            let target = address
                .checked_add(offset)
                .ok_or(BusError::AddressOverflow { address, length: 4 })?;
            self.write8(target, *byte)?;
        }
        Ok(())
    }

    /// Reads a byte range for debugger consumers.
    pub fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BusError> {
        let mut current =
            u32::try_from(address).map_err(|_| BusError::UnmappedRead { address: u32::MAX })?;
        let mut output = Vec::with_capacity(length);
        for _ in 0..length {
            output.push(self.read8(current)?);
            current = current.checked_add(1).ok_or(BusError::AddressOverflow {
                address: current,
                length,
            })?;
        }
        Ok(output)
    }

    /// Writes a byte range to work RAM for debugger consumers.
    ///
    /// The whole range is validated before the first byte is written.
    pub fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BusError> {
        let start =
            u32::try_from(address).map_err(|_| BusError::UnmappedWrite { address: u32::MAX })?;
        self.ensure_ram_write_range(start, data.len())?;
        for (offset, byte) in data.iter().enumerate() {
            let offset =
                u32::try_from(offset).map_err(|_| BusError::UnmappedWrite { address: start })?;
            let target = start.checked_add(offset).ok_or(BusError::AddressOverflow {
                address: start,
                length: data.len(),
            })?;
            self.write8(target, *byte)?;
        }
        Ok(())
    }

    /// Replaces the complete RAM contents during a snapshot restore.
    pub(crate) fn restore_ram(&mut self, ram: Vec<u8>) -> Result<(), BusError> {
        if ram.len() != RAM_SIZE {
            return Err(BusError::UnmappedWrite { address: RAM_BASE });
        }
        self.ram = ram;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{MegadriveBus, RAM_BASE};

    fn bus() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0] = 0x12;
        rom[1] = 0x34;
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    #[test]
    fn reads_rom_and_ram_big_endian() {
        let mut bus = bus();
        assert_eq!(bus.read8(0).expect("rom byte"), 0x12);
        assert_eq!(bus.read16(0).expect("rom word"), 0x1234);
        bus.write16(RAM_BASE, 0xABCD).expect("ram write");
        assert_eq!(bus.read16(RAM_BASE).expect("ram read"), 0xABCD);
        assert_eq!(bus.read8(RAM_BASE).expect("ram high"), 0xAB);
        assert_eq!(bus.read8(RAM_BASE + 1).expect("ram low"), 0xCD);
    }

    #[test]
    fn rejects_odd_word_access_and_rom_writes() {
        let mut bus = bus();
        assert!(bus.read16(1).is_err());
        assert!(bus.write16(1, 0).is_err());
        assert!(bus.write8(0, 0).is_err());
    }

    #[test]
    fn rejects_unmapped_access() {
        let bus = bus();
        assert!(bus.read8(0x00C0_0000).is_err());
        assert!(bus.read8(0x00FF_FFFF).is_ok());
    }

    #[test]
    fn invalid_write_range_does_not_partially_mutate_ram() {
        let mut bus = bus();
        let address = super::RAM_BASE + super::RAM_SIZE as u32 - 1;
        assert!(bus.write16(address, 0xABCD).is_err());
        assert_eq!(bus.read8(address).expect("last ram byte"), 0);
    }

    #[test]
    fn debugger_memory_round_trips_ram_window() {
        let mut bus = bus();
        bus.write_memory(u64::from(RAM_BASE), &[1, 2, 3, 4])
            .expect("write window");
        assert_eq!(
            bus.read_memory(u64::from(RAM_BASE), 4)
                .expect("read window"),
            vec![1, 2, 3, 4]
        );
    }
}
