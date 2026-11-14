//! Big-endian Mega Drive memory bus for the real-hardware slice.
//!
//! The cartridge ROM window, the 64 KiB work RAM window and the VDP port window
//! at `0xC00000..=0xC0000F` are mapped. PSG, Z80, controller and mapper windows
//! remain deliberately absent from this slice; accessing them is an explicit
//! error rather than silent open-bus behavior.
//!
//! VDP accesses are side-effecting (the data port advances the internal address),
//! so the bus read methods take `&mut self`. Debugger-style reads use
//! [`MegadriveBus::read_memory`], which is side-effect free and therefore rejects
//! the side-effecting VDP window instead of silently changing device state.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::vdp::{Vdp, VdpError};

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
/// First address of the VDP port window.
pub const VDP_BASE: u32 = 0x00C0_0000;
/// Size of the VDP port window in bytes.
pub const VDP_WINDOW_SIZE: u32 = 0x0010;
/// Exclusive end of the VDP port window.
pub const VDP_END: u32 = VDP_BASE + VDP_WINDOW_SIZE;

/// A bus access that the current slice cannot satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BusError {
    /// A read outside ROM, RAM or a mapped device window.
    UnmappedRead {
        /// Requested address.
        address: u32,
    },
    /// A write outside ROM, RAM or a mapped device window.
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
    /// The VDP rejected a port access.
    Vdp(VdpError),
    /// A side-effect-free debugger read targeted a side-effecting device port.
    SideEffectingRead {
        /// Requested address.
        address: u32,
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
            Self::Vdp(error) => write!(f, "VDP port access failed: {error}"),
            Self::SideEffectingRead { address } => write!(
                f,
                "side-effect-free read at 0x{address:06X} targets a side-effecting device port"
            ),
        }
    }
}

impl Error for BusError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Vdp(error) => Some(error),
            _ => None,
        }
    }
}

/// ROM + 64 KiB work RAM + the VDP port window, accessed big-endian like the
/// 68000 bus.
#[derive(Clone, Debug)]
pub struct MegadriveBus {
    rom: Arc<[u8]>,
    ram: Vec<u8>,
    vdp: Vdp,
}

impl MegadriveBus {
    /// Creates a bus over an immutable cartridge image with a VDP in its
    /// deterministic reset state.
    ///
    /// The factory validates the image size before constructing a bus.
    #[must_use]
    pub fn new(rom: Arc<[u8]>) -> Self {
        Self {
            rom,
            ram: vec![0; RAM_SIZE],
            vdp: Vdp::new(),
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

    /// Returns the VDP state.
    #[must_use]
    pub const fn vdp(&self) -> &Vdp {
        &self.vdp
    }

    /// Returns the VDP state for snapshot restore and scheduler synchronization.
    pub(crate) const fn vdp_mut(&mut self) -> &mut Vdp {
        &mut self.vdp
    }

    /// Reads one byte through the bus, including side-effecting VDP ports.
    pub fn read8(&mut self, address: u32) -> Result<u8, BusError> {
        if let Some(offset) = vdp_offset(address) {
            return self.vdp.read8(offset).map_err(BusError::Vdp);
        }
        self.peek8(address)
    }

    /// Reads one byte without side effects.
    ///
    /// The VDP window is rejected because a read there can advance the VDP's
    /// internal address; use [`MegadriveBus::read8`] when emulated execution is
    /// allowed to observe device side effects.
    pub fn peek8(&self, address: u32) -> Result<u8, BusError> {
        if vdp_offset(address).is_some() {
            return Err(BusError::SideEffectingRead { address });
        }
        if address < ROM_END {
            let index = usize::try_from(address).map_err(|_| BusError::UnmappedRead { address })?;
            return self
                .rom
                .get(index)
                .copied()
                .ok_or(BusError::UnmappedRead { address });
        }
        if (RAM_BASE..RAM_END).contains(&address) {
            // Masking to 16 bits proves the offset fits `usize` on every
            // supported target, so the conversion is infallible.
            let offset = usize::from((address & 0xFFFF) as u16);
            return self
                .ram
                .get(offset)
                .copied()
                .ok_or(BusError::UnmappedRead { address });
        }
        Err(BusError::UnmappedRead { address })
    }

    /// Reads a big-endian word through the bus.
    pub fn read16(&mut self, address: u32) -> Result<u16, BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            return self.vdp.read16(offset).map_err(BusError::Vdp);
        }
        let high = self.read8(address)?;
        let low = self.read8(
            address
                .checked_add(1)
                .ok_or(BusError::AddressOverflow { address, length: 2 })?,
        )?;
        Ok(u16::from_be_bytes([high, low]))
    }

    /// Reads a big-endian long word through the bus.
    pub fn read32(&mut self, address: u32) -> Result<u32, BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            let second_address = address
                .checked_add(2)
                .ok_or(BusError::AddressOverflow { address, length: 4 })?;
            let second_offset = vdp_offset(second_address).ok_or(BusError::UnmappedRead {
                address: second_address,
            })?;
            let high = u32::from(self.vdp.read16(offset).map_err(BusError::Vdp)?);
            let low = u32::from(self.vdp.read16(second_offset).map_err(BusError::Vdp)?);
            return Ok((high << 16) | low);
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

    /// Writes one byte to a mapped window.
    pub fn write8(&mut self, address: u32, value: u8) -> Result<(), BusError> {
        if let Some(offset) = vdp_offset(address) {
            return self.vdp.write8(offset, value).map_err(BusError::Vdp);
        }
        self.ensure_ram_write_range(address, 1)?;
        // As in `peek8`, the 16-bit mask makes the offset conversion total.
        let offset = usize::from((address & 0xFFFF) as u16);
        if let Some(slot) = self.ram.get_mut(offset) {
            *slot = value;
            return Ok(());
        }
        Err(BusError::UnmappedWrite { address })
    }

    /// Writes a big-endian word to a mapped window.
    pub fn write16(&mut self, address: u32, value: u16) -> Result<(), BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            return self.vdp.write16(offset, value).map_err(BusError::Vdp);
        }
        self.ensure_ram_write_range(address, 2)?;
        let bytes = value.to_be_bytes();
        self.write8(address, bytes[0])?;
        let second = address
            .checked_add(1)
            .ok_or(BusError::AddressOverflow { address, length: 2 })?;
        self.write8(second, bytes[1])
    }

    /// Writes a big-endian long word to a mapped window.
    pub fn write32(&mut self, address: u32, value: u32) -> Result<(), BusError> {
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            let second_address = address
                .checked_add(2)
                .ok_or(BusError::AddressOverflow { address, length: 4 })?;
            let second_offset = vdp_offset(second_address).ok_or(BusError::UnmappedWrite {
                address: second_address,
            })?;
            let bytes = value.to_be_bytes();
            self.vdp
                .write16(offset, u16::from_be_bytes([bytes[0], bytes[1]]))
                .map_err(BusError::Vdp)?;
            return self
                .vdp
                .write16(second_offset, u16::from_be_bytes([bytes[2], bytes[3]]))
                .map_err(BusError::Vdp);
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

    /// Returns the side-effect-free window that contains `start`, together with
    /// the window's first bus address.
    ///
    /// Debugger reads may target the cartridge image or work RAM. The VDP port
    /// window is excluded because reading it changes device state.
    fn debugger_window(&self, start: u32) -> Option<(&[u8], u32)> {
        // Only the first `ROM_END` bytes of the image are addressable on the
        // 68000 bus; a larger image never reaches the bus through this window.
        let rom_len = self.rom.len().min(ROM_END as usize);
        if (start as usize) < rom_len {
            return Some((&self.rom[..rom_len], ROM_BASE));
        }
        if (RAM_BASE..RAM_END).contains(&start) {
            return Some((self.ram.as_slice(), RAM_BASE));
        }
        None
    }

    /// Returns the error for a debugger access that starts outside ROM and RAM.
    fn unmapped_debugger_access(&self, address: u32) -> BusError {
        if vdp_offset(address).is_some() {
            BusError::SideEffectingRead { address }
        } else {
            BusError::UnmappedRead { address }
        }
    }

    /// Reads a byte range for debugger consumers without device side effects.
    ///
    /// The VDP port window is rejected rather than read, because a VDP read can
    /// change device state. This keeps inspection side-effect free.
    ///
    /// The complete range is validated against its mapped window before the
    /// output buffer is allocated: the cartridge image or work RAM must contain
    /// the whole request. A caller-controlled `length` that is enormous or that
    /// overflows the address space therefore returns a typed error instead of
    /// requesting an allocation that could abort the process.
    pub fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BusError> {
        // The start address is validated before the zero-length shortcut: the
        // baseline rejected addresses above the 32-bit bus even for an empty
        // read, while any representable address with length zero is accepted
        // without touching a window.
        let start =
            u32::try_from(address).map_err(|_| BusError::UnmappedRead { address: u32::MAX })?;
        if length == 0 {
            return Ok(Vec::new());
        }
        let (window, base) = self
            .debugger_window(start)
            .ok_or_else(|| self.unmapped_debugger_access(start))?;
        let offset = u64::from(start - base);
        let end = offset
            .checked_add(
                u64::try_from(length).map_err(|_| BusError::AddressOverflow {
                    address: start,
                    length,
                })?,
            )
            .ok_or(BusError::AddressOverflow {
                address: start,
                length,
            })?;
        if end > window.len() as u64 {
            // Report the first byte that the window cannot serve. A window end
            // beyond the 32-bit bus is itself an address overflow.
            let window_end = u64::from(base) + window.len() as u64;
            return Err(match u32::try_from(window_end) {
                Ok(address) => BusError::UnmappedRead { address },
                Err(_) => BusError::AddressOverflow {
                    address: u32::MAX,
                    length,
                },
            });
        }
        // The range proof above leaves both bounds inside `window`, whose
        // length fits `usize`.
        Ok(window[offset as usize..end as usize].to_vec())
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

fn vdp_offset(address: u32) -> Option<u8> {
    let offset = address.checked_sub(VDP_BASE)?;
    if offset < VDP_WINDOW_SIZE {
        u8::try_from(offset).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{BusError, MegadriveBus, RAM_BASE};

    fn bus() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0] = 0x12;
        rom[1] = 0x34;
        MegadriveBus::new(Arc::from(rom.into_boxed_slice()))
    }

    fn set_vram_address(bus: &mut MegadriveBus, address: u16, first_word: u16) {
        bus.write16(0x00C0_0004, first_word | (address & 0x3FFF))
            .expect("control first word");
        bus.write16(0x00C0_0004, (address >> 14) & 0x0003)
            .expect("control second word");
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
        let mut bus = bus();
        assert!(matches!(
            bus.read8(0x00C0_0010),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(matches!(
            bus.read8(0x00A0_0000),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(bus.read8(0x00FF_FFFF).is_ok());
    }

    #[test]
    fn vdp_control_data_and_status_are_mapped_through_the_bus() {
        let mut bus = bus();
        bus.write16(0x00C0_0004, 0x8F02).expect("auto-increment");
        assert_eq!(bus.vdp().register(15), Some(2));

        set_vram_address(&mut bus, 0x0010, 0x4000);
        bus.write16(0x00C0_0000, 0xABCD).expect("vram write");
        assert_eq!(&bus.vdp().vram()[0x10..0x12], &[0xAB, 0xCD]);

        set_vram_address(&mut bus, 0x0010, 0x0000);
        assert_eq!(bus.read16(0x00C0_0000).expect("vram read"), 0xABCD);

        // A byte read still transfers a whole word, so each lane needs its own
        // address setup; the selected byte lane comes from the port offset.
        set_vram_address(&mut bus, 0x0010, 0x0000);
        assert_eq!(bus.read8(0x00C0_0000).expect("high byte"), 0xAB);
        set_vram_address(&mut bus, 0x0010, 0x0000);
        assert_eq!(bus.read8(0x00C0_0001).expect("low byte"), 0xCD);

        // Data-port byte writes duplicate into both halves.
        set_vram_address(&mut bus, 0x0020, 0x4000);
        bus.write8(0x00C0_0000, 0x5A).expect("byte write");
        assert_eq!(&bus.vdp().vram()[0x20..0x22], &[0x5A, 0x5A]);

        assert_eq!(bus.read16(0x00C0_0004).expect("status"), 0x0100);
        assert_eq!(bus.read16(0x00C0_0008).expect("hv counter"), 0);
    }

    #[test]
    fn vdp_port_errors_and_other_devices_are_explicit() {
        let mut bus = bus();
        assert!(matches!(
            bus.read16(0x00C0_0001),
            Err(BusError::OddAddress { .. })
        ));
        bus.write16(0x00C0_0004, 0x8F02).expect("auto-increment");
        assert!(matches!(
            bus.write16(0x00C0_0004, 0x9800),
            Err(BusError::Vdp(_))
        ));
        assert!(matches!(
            bus.read8(0x00C0_0011),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(matches!(
            bus.write8(0x00C0_0011, 0),
            Err(BusError::UnmappedWrite { .. })
        ));
        assert!(matches!(
            bus.read8(0x00A1_0000),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(matches!(
            bus.write16(0x00A1_0000, 0),
            Err(BusError::UnmappedWrite { .. })
        ));
    }

    #[test]
    fn debugger_reads_of_vdp_ports_are_side_effect_free() {
        let mut bus = bus();
        set_vram_address(&mut bus, 0x0010, 0x4000);
        bus.write16(0x00C0_0000, 0xABCD).expect("vram write");
        set_vram_address(&mut bus, 0x0010, 0x0000);
        let address_before = bus.vdp().address();
        assert_eq!(
            bus.read_memory(0x00C0_0000, 1),
            Err(BusError::SideEffectingRead {
                address: 0x00C0_0000
            })
        );
        assert_eq!(bus.vdp().address(), address_before);
        assert_eq!(
            bus.read_memory(u64::from(RAM_BASE), 2).expect("ram peek"),
            vec![0, 0]
        );
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

    #[test]
    fn zero_length_debugger_reads_still_validate_the_start_address() {
        let bus = bus();
        assert_eq!(
            bus.read_memory(0, 0).expect("empty ROM read"),
            Vec::<u8>::new()
        );
        // A representable address is accepted without touching a window, so a
        // side-effecting port is fine for an empty read (baseline behavior).
        assert_eq!(
            bus.read_memory(0x00C0_0000, 0).expect("empty VDP read"),
            Vec::<u8>::new()
        );
        // An address outside the 32-bit bus is rejected even for length zero.
        assert_eq!(
            bus.read_memory(u64::from(u32::MAX) + 1, 0),
            Err(BusError::UnmappedRead { address: u32::MAX })
        );
    }

    #[test]
    fn debugger_reads_validate_the_range_before_allocating() {
        let bus = bus();
        assert_eq!(bus.read_memory(0, 2).expect("rom read"), vec![0x12, 0x34]);
        // The last RAM byte exists; one byte more is reported at the first
        // address the window cannot serve.
        let last_ram = u64::from(RAM_BASE) + u64::try_from(super::RAM_SIZE).expect("ram size") - 1;
        assert_eq!(
            bus.read_memory(last_ram, 2),
            Err(BusError::UnmappedRead {
                address: super::RAM_END
            })
        );
        // A huge length is rejected by range arithmetic, never by asking the
        // allocator for it.
        assert_eq!(
            bus.read_memory(u64::from(RAM_BASE), usize::MAX),
            Err(BusError::UnmappedRead {
                address: super::RAM_END
            })
        );
        assert_eq!(
            bus.read_memory(u64::from(RAM_BASE) + 1, usize::MAX),
            Err(BusError::AddressOverflow {
                address: RAM_BASE + 1,
                length: usize::MAX
            })
        );
        // Reading past the cartridge image inside the ROM window is unmapped,
        // and addresses above the 32-bit bus cannot be expressed at all.
        assert_eq!(
            bus.read_memory(0x400, 1),
            Err(BusError::UnmappedRead { address: 0x400 })
        );
        assert_eq!(
            bus.read_memory(u64::from(u32::MAX) + 1, 1),
            Err(BusError::UnmappedRead { address: u32::MAX })
        );
    }
}
