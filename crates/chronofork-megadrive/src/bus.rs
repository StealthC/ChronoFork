//! Big-endian Mega Drive memory bus for the real-hardware slice.
//!
//! The cartridge windows (ROM, mapper registers, battery SRAM and TMSS), the
//! 64 KiB work RAM window and the VDP port window at `0xC00000..=0xC0000F` are
//! mapped and delegated to [`Cartridge`]. The version register
//! at `0xA10001` reports the cartridge's header/region/video facts. The two
//! controller data registers at
//! `0xA10003` (port 1) and `0xA10005` (port 2) are mapped for **execution byte
//! reads only**, and only while a step is executing: they return a TH-high
//! 3-button subset derived from the current step's `InputFrame`. Every other
//! controller/IO-window access, every controller write, and every debugger read of
//! the window is an explicit typed error rather than silent open-bus behavior.
//! PSG and Z80 windows remain deliberately absent from this slice.
//!
//! VDP accesses are side-effecting (the data port advances the internal address),
//! so the bus read methods take `&mut self`. Debugger-style reads use
//! [`MegadriveBus::read_memory`], which is side-effect free and therefore rejects
//! the side-effecting VDP window instead of silently changing device state. The
//! controller value is step-scoped rather than machine state, so the debugger path
//! rejects it too.
//!
//! # Debug observation
//!
//! Every emulated bus access passes through this type, which makes it the single
//! observation point for debug events (see the private `debug` module). An
//! optional observer records one watchpoint event per matching byte lane plus
//! typed bus/device events; it is off by default and never changes emulated
//! behavior. The debugger paths [`MegadriveBus::read_memory`] and
//! [`MegadriveBus::write_memory`] deliberately bypass the observer, because they
//! are not emulated bus accesses.

use std::error::Error;
use std::fmt;

use chronofork_backend_api::{DebugConfig, DebugEventKind, DebugEventLog, WatchpointAccess};
use chronofork_core::Tick;

use crate::cartridge::{
    Cartridge, CartridgeError, MAPPER_REGISTER_BASE, MAPPER_REGISTER_END, TMSS_REGISTER_BASE,
    TMSS_REGISTER_END, Tmss, VERSION_REGISTER,
};
use crate::debug::DebugObserver;
use crate::interrupts::VideoStandard;
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
/// First address of the Mega Drive controller/IO window.
pub const CONTROLLER_WINDOW_BASE: u32 = 0x00A1_0000;
/// Exclusive end of the controller/IO window.
pub const CONTROLLER_WINDOW_END: u32 = 0x00A1_0020;
/// Controller port 1 data register.
pub const CONTROLLER_PORT_1: u32 = 0x00A1_0003;
/// Controller port 2 data register.
pub const CONTROLLER_PORT_2: u32 = 0x00A1_0005;
/// Data-register byte read when no button is pressed and TH is high.
///
/// Bit 6 is the TH-high level and bit 7 is the slice's constant zero; the six
/// active-low button bits are all released.
pub const CONTROLLER_NEUTRAL: u8 = 0x7F;

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
    /// The cartridge device rejected an access that is not one of the mapped
    /// read/write/protection errors above.
    Cartridge(CartridgeError),
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
            Self::Cartridge(error) => write!(f, "cartridge access failed: {error}"),
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
            Self::Cartridge(error) => Some(error),
            _ => None,
        }
    }
}

/// Cartridge device + 64 KiB work RAM + the VDP port window, accessed big-endian
/// like the 68000 bus.
#[derive(Clone, Debug)]
pub struct MegadriveBus {
    cartridge: Cartridge,
    /// Video standard the machine presents, used by the cartridge version
    /// register. This is configuration, not mutable state: it is covered by the
    /// configuration hash and never serialized.
    video: VideoStandard,
    ram: Vec<u8>,
    vdp: Vdp,
    /// Data-register bytes for controller ports 1 and 2 during the instruction
    /// currently executing.
    ///
    /// This is transient, step-scoped input, not canonical machine state: it is
    /// not serialized, hashed or inspected, and it is neutral whenever no step is
    /// executing. [`MegadriveBackend::step`](crate::MegadriveBackend) sets it at
    /// step entry and clears it on every exit path.
    controller_ports: [u8; 2],
    /// Debug-only observation state.
    ///
    /// Never canonical: absent from snapshots, state hashes and configuration
    /// hashes, and a restore does not rewind it. It is off (no event ring, no
    /// watchpoints) until debug observation is configured, so an ordinary
    /// machine pays one branch per bus byte access and stores nothing.
    debug: DebugObserver,
}

impl MegadriveBus {
    /// Creates a bus over a parsed cartridge with a VDP in its deterministic
    /// reset state.
    ///
    /// The video standard is derived from the cartridge header region.
    #[must_use]
    pub fn new(cartridge: Cartridge) -> Self {
        let video = cartridge.derived_video_standard();
        Self::with_video_standard(cartridge, video)
    }

    /// Creates a bus over a parsed cartridge with an explicit video standard.
    ///
    /// The factory uses this to apply a [`MegadriveConfig`](crate::MegadriveConfig)
    /// override; the value feeds the version register and must match the
    /// scheduler standard.
    #[must_use]
    pub fn with_video_standard(cartridge: Cartridge, video: VideoStandard) -> Self {
        Self {
            cartridge,
            video,
            ram: vec![0; RAM_SIZE],
            vdp: Vdp::new(),
            controller_ports: [CONTROLLER_NEUTRAL; 2],
            debug: DebugObserver::default(),
        }
    }

    /// Installs a validated debug observation configuration.
    ///
    /// Any previously retained events are discarded and observation starts from
    /// the current machine state. This is debug-only state: canonical state,
    /// snapshots, the state hash and the configuration hash are untouched.
    ///
    /// # Errors
    ///
    /// Returns a typed [`BackendError`](chronofork_backend_api::BackendError)
    /// when the event ring cannot be created.
    pub(crate) fn debug_configure(
        &mut self,
        config: DebugConfig,
    ) -> Result<(), chronofork_backend_api::BackendError> {
        self.debug.configure(config)
    }

    /// Returns the installed event ring, or `None` while observation is off.
    pub(crate) fn debug_event_log(&self) -> Option<&DebugEventLog> {
        self.debug.event_log()
    }

    /// Removes the installed event ring, disabling further recording.
    pub(crate) fn take_debug_event_log(&mut self) -> Option<DebugEventLog> {
        self.debug.take_event_log()
    }

    /// Marks the start of a backend step so events carry its pre-step tick.
    pub(crate) fn debug_begin_step(&mut self, tick: Tick) {
        self.debug.begin_step(tick);
    }

    /// Returns the observer for the backend's interrupt-event bookkeeping.
    pub(crate) fn debug_observer_mut(&mut self) -> &mut DebugObserver {
        &mut self.debug
    }

    /// Records one VDP port event and any matching watchpoints.
    ///
    /// `value` is the byte or word the access transferred, so a word access
    /// reports both its byte lanes to a watchpoint and one width-2 device event.
    fn observe_vdp_access(&mut self, offset: u8, address: u32, value: u32, width: u8, write: bool) {
        self.debug.record(DebugEventKind::VdpPort {
            offset,
            value,
            width,
            write,
        });
        self.debug.observe_watchpoints(
            if write {
                WatchpointAccess::Write
            } else {
                WatchpointAccess::Read
            },
            address,
            value,
            width,
        );
    }

    /// Records one work-RAM byte write and any matching watchpoints.
    fn observe_ram_write(&mut self, address: u32, value: u8) {
        self.debug.record(DebugEventKind::BusWrite {
            address,
            value: u32::from(value),
            width: 1,
        });
        self.debug
            .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
    }

    /// Makes one step's controller port masks visible to execution byte reads.
    ///
    /// `ports[0]` drives controller port 1 and `ports[1]` port 2. Each mask uses
    /// the TH-high 3-button layout: bits 0-5 are Up, Down, Left, Right, B and C
    /// (set = pressed, active-low in the data register); A (bit 6), Start (bit 7)
    /// and every higher bit are inert. The derived bytes are cleared by
    /// [`MegadriveBus::clear_step_input`] when the step ends.
    pub(crate) fn set_step_input(&mut self, ports: [u32; 2]) {
        self.controller_ports = ports.map(controller_data_byte);
    }

    /// Returns the controller data registers to their neutral idle bytes.
    ///
    /// The backend calls this on every `step` exit path, so a direct bus read
    /// outside a step never observes the previous step's input.
    pub(crate) fn clear_step_input(&mut self) {
        self.controller_ports = [CONTROLLER_NEUTRAL; 2];
    }

    /// Returns the cartridge device.
    #[must_use]
    pub const fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    /// Returns the cartridge device for snapshot restore and test/debug writes.
    #[must_use]
    pub const fn cartridge_mut(&mut self) -> &mut Cartridge {
        &mut self.cartridge
    }

    /// Returns the TMSS latch owned by the cartridge.
    #[must_use]
    pub const fn tmss(&self) -> &Tmss {
        self.cartridge.tmss()
    }

    /// Returns the cartridge image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        self.cartridge.rom()
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

    /// Takes the VDP FIFO wait slots accumulated since the previous call.
    ///
    /// The count is in 68000 clock cycles; the backend converts it to master
    /// cycles (`wait * 7`) and adds it to the instruction cost. A write that
    /// found the four-word FIFO full contributes one slot and is never dropped.
    pub fn take_vdp_wait_cycles(&mut self) -> u32 {
        self.vdp.take_fifo_wait_cycles()
    }

    /// Reads one byte through the bus, including side-effecting VDP ports and the
    /// step-scoped controller data registers.
    pub fn read8(&mut self, address: u32) -> Result<u8, BusError> {
        if let Some(offset) = vdp_offset(address) {
            let value = self.vdp.read8(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 1, false);
            return Ok(value);
        }
        if let Some((slot, port)) = controller_port(address) {
            let value = self.controller_ports[slot];
            self.debug
                .record(DebugEventKind::ControllerRead { port, value });
            self.debug
                .observe_watchpoints(WatchpointAccess::Read, address, u32::from(value), 1);
            return Ok(value);
        }
        let value = self.peek8(address)?;
        self.debug
            .observe_watchpoints(WatchpointAccess::Read, address, u32::from(value), 1);
        Ok(value)
    }

    /// Reads one byte without side effects.
    ///
    /// The VDP window is rejected because a read there can advance the VDP's
    /// internal address; use [`MegadriveBus::read8`] when emulated execution is
    /// allowed to observe device side effects. Cartridge reads (ROM, mapper
    /// registers, mapped SRAM and the version register) are side-effect free and
    /// never mutate mapper or SRAM state.
    pub fn peek8(&self, address: u32) -> Result<u8, BusError> {
        if vdp_offset(address).is_some() {
            return Err(BusError::SideEffectingRead { address });
        }
        if controller_port(address).is_some() {
            return Err(BusError::UnmappedRead { address });
        }
        if address == VERSION_REGISTER {
            return Ok(self.cartridge.version_register(self.video));
        }
        if is_cartridge_address(address) {
            return self.cartridge.read8(address).map_err(map_cartridge_read);
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
            let value = self.vdp.read16(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 2, false);
            return Ok(value);
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
            let high = self.vdp.read16(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(high), 2, false);
            let low = self.vdp.read16(second_offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(second_offset, second_address, u32::from(low), 2, false);
            return Ok((u32::from(high) << 16) | u32::from(low));
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

    /// Validates that a write range lies entirely in work RAM.
    ///
    /// Exception entry uses this to reject an invalid frame before any byte is
    /// written, so a failed push cannot leave a partial frame behind.
    pub fn validate_write_range(&self, address: u32, length: usize) -> Result<(), BusError> {
        self.ensure_ram_write_range(address, length)
    }

    /// Writes one byte to a mapped window.
    ///
    /// While the enabled TMSS lockout is active a VDP write is acknowledged and
    /// discarded: neither the VDP registers nor its address latch change.
    pub fn write8(&mut self, address: u32, value: u8) -> Result<(), BusError> {
        if let Some(offset) = vdp_offset(address) {
            if !self.cartridge.tmss().vdp_writes_allowed() {
                return Ok(());
            }
            self.vdp.write8(offset, value).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 1, true);
            return Ok(());
        }
        if is_cartridge_address(address) {
            self.cartridge
                .write8(address, value)
                .map_err(map_cartridge_write)?;
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        self.write_ram_byte(address, value)?;
        self.observe_ram_write(address, value);
        Ok(())
    }

    /// Writes one byte to work RAM without debug observation.
    ///
    /// Emulated execution reaches this through [`MegadriveBus::write8`], which
    /// adds the debug events. Debugger writes use it directly so that
    /// [`MegadriveBus::write_memory`] never produces an event.
    fn write_ram_byte(&mut self, address: u32, value: u8) -> Result<(), BusError> {
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
            if !self.cartridge.tmss().vdp_writes_allowed() {
                return Ok(());
            }
            self.vdp.write16(offset, value).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 2, true);
            return Ok(());
        }
        if is_cartridge_address(address) {
            self.cartridge
                .write16(address, value)
                .map_err(map_cartridge_write)?;
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 2);
            return Ok(());
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
            if !self.cartridge.tmss().vdp_writes_allowed() {
                return Ok(());
            }
            let second_address = address
                .checked_add(2)
                .ok_or(BusError::AddressOverflow { address, length: 4 })?;
            let second_offset = vdp_offset(second_address).ok_or(BusError::UnmappedWrite {
                address: second_address,
            })?;
            let bytes = value.to_be_bytes();
            let high = u16::from_be_bytes([bytes[0], bytes[1]]);
            self.vdp.write16(offset, high).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(high), 2, true);
            let low = u16::from_be_bytes([bytes[2], bytes[3]]);
            self.vdp
                .write16(second_offset, low)
                .map_err(BusError::Vdp)?;
            self.observe_vdp_access(second_offset, second_address, u32::from(low), 2, true);
            return Ok(());
        }
        if is_cartridge_address(address) {
            self.cartridge
                .write32(address, value)
                .map_err(map_cartridge_write)?;
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, value, 4);
            return Ok(());
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
        let rom = self.cartridge.rom();
        let rom_len = rom.len().min(ROM_END as usize);
        if (start as usize) < rom_len {
            return Some((&rom[..rom_len], ROM_BASE));
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
    /// The whole range is validated before the first byte is written. This is
    /// not an emulated bus write, so it never produces a debug event: it writes
    /// work RAM directly instead of going through [`MegadriveBus::write8`].
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
            self.write_ram_byte(target, *byte)?;
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

/// Returns whether `address` belongs to a cartridge-owned window.
///
/// This is the complete cartridge address surface: the ROM/SRAM window, the
/// mapper register file and the TMSS register. The bus still rejects every other
/// `0xA1xxxx` address as unmapped.
fn is_cartridge_address(address: u32) -> bool {
    address < ROM_END
        || (MAPPER_REGISTER_BASE..MAPPER_REGISTER_END).contains(&address)
        || (TMSS_REGISTER_BASE..=TMSS_REGISTER_END).contains(&address)
}

/// Maps a cartridge read failure to the bus contract.
fn map_cartridge_read(error: CartridgeError) -> BusError {
    match error {
        CartridgeError::UnmappedRom { address } => BusError::UnmappedRead { address },
        CartridgeError::OddAddress { address } => BusError::OddAddress { address },
        CartridgeError::WriteProtected { address } => BusError::WriteProtected { address },
        other => BusError::Cartridge(other),
    }
}

/// Maps a cartridge write failure to the bus contract.
fn map_cartridge_write(error: CartridgeError) -> BusError {
    match error {
        CartridgeError::UnmappedRom { address } => BusError::UnmappedWrite { address },
        CartridgeError::OddAddress { address } => BusError::OddAddress { address },
        CartridgeError::WriteProtected { address } => BusError::WriteProtected { address },
        other => BusError::Cartridge(other),
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

/// Returns the step-scoped input slot and Mega Drive port number addressed by
/// `address`, if any.
///
/// Only the two data-register bytes are mapped; the version, control, TX/RX and
/// port-3 addresses are not. The port number is the hardware-facing one (`1` for
/// `0xA10003`, `2` for `0xA10005`) and is what
/// [`DebugEventKind::ControllerRead`] reports.
fn controller_port(address: u32) -> Option<(usize, u8)> {
    match address {
        CONTROLLER_PORT_1 => Some((0, 1)),
        CONTROLLER_PORT_2 => Some((1, 2)),
        _ => None,
    }
}

/// Derives one TH-high 3-button data-register byte from a port mask.
///
/// Bits 0-5 are Up, Down, Left, Right, B and C, active-low in the data register.
/// Bit 6 reads as the TH-high level, bit 7 as the slice's constant zero. Bits 6
/// and above of the input mask (A, Start and anything else) have no effect.
fn controller_data_byte(port: u32) -> u8 {
    // The low six bits are the whole mapped subset, so the mask fits `u8`.
    let pressed = (port & 0x3F) as u8;
    0x40 | (!pressed & 0x3F)
}

#[cfg(test)]
mod tests {
    use crate::cartridge::Cartridge;

    use super::{BusError, MegadriveBus, RAM_BASE};

    fn bus() -> MegadriveBus {
        let mut rom = vec![0_u8; 0x400];
        rom[0] = 0x12;
        rom[1] = 0x34;
        MegadriveBus::new(test_cartridge(rom))
    }

    /// Stamps a valid synthetic header on a raw test image.
    fn test_cartridge(rom: Vec<u8>) -> Cartridge {
        crate::cartridge::test_cartridge(rom)
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
    fn controller_ports_read_the_th_high_three_button_subset() {
        let mut bus = bus();
        // Neutral, each documented button bit, and the all-pressed combination.
        let cases = [
            (0x00, 0x7F), // released
            (0x01, 0x7E), // Up
            (0x02, 0x7D), // Down
            (0x04, 0x7B), // Left
            (0x08, 0x77), // Right
            (0x10, 0x6F), // B
            (0x20, 0x5F), // C
            (0x11, 0x6E), // Up + B
            (0x3F, 0x40), // every mapped button pressed
        ];
        for (mask, expected) in cases {
            bus.set_step_input([mask, mask]);
            assert_eq!(
                bus.read8(super::CONTROLLER_PORT_1),
                Ok(expected),
                "port 1 mask 0x{mask:02X}"
            );
            assert_eq!(
                bus.read8(super::CONTROLLER_PORT_2),
                Ok(expected),
                "port 2 mask 0x{mask:02X}"
            );
        }

        // Frame bits 6 (A), 7 (Start) and every higher bit have no effect.
        bus.set_step_input([0x4000_0001, 0x8000_0020]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7E));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_2), Ok(0x5F));
        bus.set_step_input([u32::MAX, u32::MAX]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x40));
        bus.set_step_input([0xFFFF_FFC0, 0xFFFF_FFC0]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7F));
    }

    #[test]
    fn controller_ports_are_independent_and_clear_to_neutral() {
        let mut bus = bus();
        bus.set_step_input([0x01, 0x20]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7E));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_2), Ok(0x5F));

        bus.set_step_input([0x20, 0x01]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x5F));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_2), Ok(0x7E));

        bus.set_step_input([0x3F, 0x3F]);
        bus.clear_step_input();
        assert_eq!(
            bus.read8(super::CONTROLLER_PORT_1),
            Ok(super::CONTROLLER_NEUTRAL)
        );
        assert_eq!(
            bus.read8(super::CONTROLLER_PORT_2),
            Ok(super::CONTROLLER_NEUTRAL)
        );
        // Clearing an already-neutral bus is idempotent.
        bus.clear_step_input();
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7F));
    }

    #[test]
    fn controller_window_maps_only_two_execution_byte_reads() {
        let mut bus = bus();
        bus.set_step_input([0x01, 0x02]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7E));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_2), Ok(0x7D));

        // Every other address in the controller/IO window, and every write,
        // is a typed error rather than a silent skip.
        for address in super::CONTROLLER_WINDOW_BASE..super::CONTROLLER_WINDOW_END {
            if address == super::CONTROLLER_PORT_1
                || address == super::CONTROLLER_PORT_2
                || address == super::VERSION_REGISTER
            {
                continue;
            }
            assert_eq!(
                bus.read8(address),
                Err(BusError::UnmappedRead { address }),
                "read at 0x{address:06X}"
            );
            assert_eq!(
                bus.write8(address, 0),
                Err(BusError::UnmappedWrite { address }),
                "write at 0x{address:06X}"
            );
        }
        for address in [super::CONTROLLER_PORT_1, super::CONTROLLER_PORT_2] {
            assert_eq!(
                bus.write8(address, super::CONTROLLER_NEUTRAL),
                Err(BusError::UnmappedWrite { address })
            );
        }
        // The version register is a side-effect-free cartridge read.
        assert_eq!(bus.read8(super::VERSION_REGISTER), Ok(0xA1));
        assert_eq!(bus.peek8(super::VERSION_REGISTER), Ok(0xA1));

        // The debugger path stays unmapped: a step-scoped value is not a machine
        // register and must not be presented as one.
        for address in [super::CONTROLLER_PORT_1, super::CONTROLLER_PORT_2] {
            assert_eq!(bus.peek8(address), Err(BusError::UnmappedRead { address }));
            assert_eq!(
                bus.read_memory(u64::from(address), 1),
                Err(BusError::UnmappedRead { address })
            );
        }

        // A word or long access that touches the window is a typed error, not a
        // partial success: the high byte of the data register is not mapped.
        assert_eq!(
            bus.read16(0x00A1_0002),
            Err(BusError::UnmappedRead {
                address: 0x00A1_0002
            })
        );
        assert_eq!(
            bus.read32(0x00A1_0000),
            Err(BusError::UnmappedRead {
                address: 0x00A1_0000
            })
        );
        assert_eq!(
            bus.write16(0x00A1_0002, 0),
            Err(BusError::UnmappedWrite {
                address: 0x00A1_0003
            })
        );
        // An odd-address word access at the data register is rejected before the
        // read, as everywhere else on the 68000 bus.
        assert_eq!(
            bus.read16(super::CONTROLLER_PORT_1),
            Err(BusError::OddAddress {
                address: super::CONTROLLER_PORT_1
            })
        );
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

        // Two data-port writes are queued, so FIFO-empty is clear until the
        // owner advances and drains the queue.
        assert_eq!(bus.read16(0x00C0_0004).expect("status"), 0x0000);
        bus.vdp_mut().drain_fifo();
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
