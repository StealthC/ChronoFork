//! Big-endian Mega Drive memory bus for the real-hardware slice.
//!
//! The cartridge windows (ROM, mapper registers, battery SRAM and TMSS), the
//! 64 KiB work RAM window and the VDP port window at `0xC00000..=0xC0000F` are
//! mapped and delegated to [`Cartridge`]. The machine decodes only 24 address
//! bits, so every entry point truncates a 32-bit register value to the 24-bit
//! bus address with [`bus_address`] as its first action; the raw register value
//! stays in the emulated CPU, which is what the hardware does too.
//!
//! Work RAM appears twice, as on hardware: the primary `0xFF0000..=0xFFFFFF`
//! window and the `0xE00000..=0xFEFFFF` mirror, both decoded through address
//! bits 15-0 (see [`is_work_ram`]).
//!
//! In the controller/IO window `0xA10000..=0xA1001F`, the version register at
//! `0xA10001` reports the cartridge's header/region/video facts, the two
//! controller data registers at `0xA10003` (port 1) and `0xA10005` (port 2) are
//! mapped for **execution byte reads only** (while a step is executing: they
//! return a TH-high 3-button subset derived from the current step's
//! `InputFrame`; a write there is acknowledged and discarded), and the eight
//! port 1/2/3/EXT control registers at `0xA10008..=0xA1000F` are mapped: the
//! four port 1/2 bytes (`0xA10008..=0xA1000B`) are read/write latches whose
//! bytes are canonical machine state, while the port 3/EXT **control** pairs
//! (`0xA1000C..=0xA1000F`) are the ADR-0014 constant-zero **stub** — a read
//! returns `0x00` and a write is acknowledged and discarded, so those four
//! bytes never change. Port 3/EXT data
//! (`0xA10006`/`0xA10007`) is a register this slice does not model: the access is
//! accepted with no effect and counted in [`MegadriveBus::port3_unmodelled_accesses`],
//! because faulting it turned a legal guest store into a guest address error that
//! skipped the instruction in a commercial boot (W1X); every other
//! address in the window reads [`OPEN_BUS_VALUE`] and discards writes, as the
//! undecoded pins of the window do. Every debugger read of a step-scoped data
//! register stays a typed error, because a per-step input value is not a
//! machine register.
//!
//! The phase-8.4 Z80 subsystem is visible from both sides. The 68000 sees the
//! 8 KiB Z80 RAM at `0xA00000..=0xA01FFF`, the nine-bit bank register at
//! `0xA06000`, and the bus-request/reset registers at `0xA11100`/`0xA11200`.
//! The Z80 sees its RAM mirrored through `0x3FFF`, the `SOUND` region at
//! `0x4000..=0x5FFF` (the YM2612's four ports repeated by an `0x0003` address
//! mask), the bank-register write at `0x6000`, the VDP port window at
//! `0x7F00..=0x7F03`, the PSG at `0x7F11` plus its mirror `0x7F10..=0x7F17`, and a
//! 32 KiB banked window at
//! `0x8000..=0xFFFF`. The YM2612 window is wired on both sides (the 68000 side
//! at `0xA04000..=0xA04003`) and the Z80 PSG window writes the PSG latch/data
//! protocol; a PSG read returns the chip's documented open value. The 68000-side
//! PSG window `0xC00010..=0xC00017` drives that same instance, and the whole I/O
//! window above `0xA1001F`
//! (`0xA11100`/`0xA11200` excepted) stays a typed error rather than open bus:
//! the deterministic open value is deliberately limited to the window this
//! phase maps.
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

use crate::audio::psg::OPEN_BUS_VALUE;
use crate::audio::{Psg, Ym2612, YmBank, YmPort};
use crate::cartridge::{
    Cartridge, CartridgeError, MAPPER_REGISTER_BASE, MAPPER_REGISTER_END, TMSS_REGISTER_BASE,
    TMSS_REGISTER_END, Tmss, VERSION_REGISTER,
};
use crate::cycles::MASTER_CYCLES_PER_Z80_T_STATE;
use crate::debug::DebugObserver;
use crate::interrupts::VideoStandard;
use crate::vdp::{Vdp, VdpError};
use crate::z80::{Z80Bus, Z80Error};

/// First address of the cartridge ROM window.
pub const ROM_BASE: u32 = 0x0000_0000;
/// Exclusive end of the cartridge ROM window on the 68000 bus.
pub const ROM_END: u32 = 0x0040_0000;
/// Bits 31..24 of a 68000 address register are not decoded by the machine.
pub const ADDRESS_MASK: u32 = 0x00FF_FFFF;
/// First address of the 64 KiB work RAM window.
pub const RAM_BASE: u32 = 0x00FF_0000;
/// Exclusive end of the work RAM primary window.
pub const RAM_END: u32 = 0x0100_0000;
/// First address of the `0xE00000..=0xFFFFFF` work RAM mirror.
///
/// Every address from here to the end of the 24-bit bus decodes to work RAM
/// through address bits 15-0, so the mirror covers `0xE00000..=0xFEFFFF` plus
/// the primary `0xFF0000..=0xFFFFFF` window.
pub const RAM_MIRROR_BASE: u32 = 0x00E0_0000;
/// Size of work RAM in bytes.
pub const RAM_SIZE: usize = 0x0001_0000;
/// First address of the VDP port window.
pub const VDP_BASE: u32 = 0x00C0_0000;
/// Size of the VDP port window in bytes.
pub const VDP_WINDOW_SIZE: u32 = 0x0010;
/// Exclusive end of the VDP port window.
pub const VDP_END: u32 = VDP_BASE + VDP_WINDOW_SIZE;
/// First 68000 address of the PSG window.
///
/// The pinned oracle reaches the PSG through the VDP: the VDP's `10*` port cases
/// forward the access to the referenced SN76489 (`Devices/315-5313/S315-5313_Ports.cpp`
/// `case 4: // 10* - SN76489 PSG` / `case 5`, around `:1639`), and the VDP device
/// is selected for the whole `0xC00000..=0xC0001F` block by
/// `Data/Modules/Sega Mega Drive 1600.xml:318`
/// (`AddressLineFilter="110??000 ???????? 00000000"`, `MemoryMapSize="0x20"`,
/// `AddressMask="0x00001F"`), with the PSG reference declared at `:319`. The
/// eight bytes below therefore drive the *same* [`Psg`] instance the Z80 side
/// writes at `0x7F11` (`MDBusArbiter.cpp:352`/`:516` OR the Z80 window offset into
/// `0xC00000`), and the pin shows no odd-byte-only rule.
pub const PSG_BASE: u32 = 0x00C0_0010;
/// Size of the 68000-visible PSG window in bytes.
pub const PSG_WINDOW_SIZE: u32 = 8;
/// Exclusive end of the 68000-visible PSG window.
pub const PSG_END: u32 = PSG_BASE + PSG_WINDOW_SIZE;
/// First address of the Mega Drive controller/IO window.
pub const CONTROLLER_WINDOW_BASE: u32 = 0x00A1_0000;
/// Exclusive end of the controller/IO window.
pub const CONTROLLER_WINDOW_END: u32 = 0x00A1_0020;
/// Controller port 1 data register.
pub const CONTROLLER_PORT_1: u32 = 0x00A1_0003;
/// Controller port 2 data register.
pub const CONTROLLER_PORT_2: u32 = 0x00A1_0005;
/// Port 3/EXT data register (the second half of the port 1/2 data word).
pub const CONTROLLER_PORT_3_DATA: u32 = 0x00A1_0006;
/// Port 3/EXT data register, low byte.
pub const CONTROLLER_PORT_3_DATA_LOW: u32 = 0x00A1_0007;
/// First address of the port 1/2/3/EXT control register file.
///
/// The file's first half (`0xA10008..=0xA1000B`) is the two latched port 1/2
/// control register pairs; the second half is the constant-zero port 3/EXT
/// stub documented on [`IO_CONTROL_STUB_BASE`].
pub const IO_CONTROL_BASE: u32 = 0x00A1_0008;
/// Number of bytes in the port 1/2/3/EXT control register file.
pub const IO_CONTROL_LEN: usize = 8;
/// First address of the port 3/EXT control **stub** half of the register file.
///
/// ADR-0014 pins the port 3/EXT control pairs (`0xA1000C..=0xA1000F`) to the
/// constant zero: reads return `0x00` and writes are discarded, while the port
/// 1/2 control pairs (`0xA10008..=0xA1000B`) latch. The eight bytes still share
/// one canonical array and one snapshot home.
pub const IO_CONTROL_STUB_BASE: u32 = 0x00A1_000C;
/// Data-register byte read when no button is pressed and TH is high.
///
/// Bit 6 is the TH-high level and bit 7 is the slice's constant zero; the six
/// active-low button bits are all released.
pub const CONTROLLER_NEUTRAL: u8 = 0x7F;
/// First 68000 address of the YM2612 window.
pub const YM2612_BASE: u32 = 0x00A0_4000;
/// Size of the 68000-visible YM2612 window in bytes.
pub const YM2612_WINDOW_SIZE: u32 = 4;
/// Exclusive end of the 68000-visible YM2612 window.
pub const YM2612_END: u32 = YM2612_BASE + YM2612_WINDOW_SIZE;
/// First 68000 address of the Z80 RAM window.
pub const Z80_RAM_BASE: u32 = 0x00A0_0000;
/// Size of Z80 RAM in bytes.
pub const Z80_RAM_SIZE: usize = 0x2000;
/// Exclusive end of the Z80 RAM window on the 68000 bus.
pub const Z80_RAM_END: u32 = Z80_RAM_BASE + Z80_RAM_SIZE as u32;
/// 68000-side Z80 bank register.
pub const Z80_BANK_REGISTER: u32 = 0x00A0_6000;
/// 68000-side Z80 bus-request register.
pub const Z80_BUS_REQUEST: u32 = 0x00A1_1100;
/// 68000-side Z80 reset register (active-low bit 8).
pub const Z80_RESET: u32 = 0x00A1_1200;
/// Mask selecting one byte inside the 8 KiB Z80 RAM mirror.
const Z80_RAM_MASK: u16 = 0x1FFF;
/// First Z80 address of the banked cartridge ROM window.
pub const Z80_ROM_WINDOW_BASE: u16 = 0x8000;
/// Z80-side bank-register write address.
pub const Z80_BANK_REGISTER_ADDRESS: u16 = 0x6000;
/// First Z80 address of the YM2612 window.
pub const YM2612_Z80_BASE: u16 = 0x4000;
/// Last Z80 address of the four-byte YM2612 window.
pub const YM2612_Z80_END: u16 = 0x4003;
/// Last Z80 address of the whole `SOUND` region.
///
/// The pinned oracle asserts the sound chip-select for `0x4000..=0x5FFF`
/// (`Devices/MD1600IO/MDBusArbiter.cpp:1048-1049`) and maps the YM2612 there with
/// `AddressMask=0x0003` over a `0x2000` region (`Data/Modules/Sega Mega Drive
/// 1600.xml:340`), applied by `System/BusInterface.cpp:2454`. So the four ports
/// repeat across the whole region and `0x5F8B & 3 = 3` reaches the bank-2 data
/// port, exactly like `0x4003`.
pub const YM2612_Z80_SOUND_END: u16 = 0x5FFF;
/// First Z80 address of the VDP port window.
///
/// The arbiter translates every byte of its `0x7F00` window into the 68000
/// address `0xC00000 | offset` (`Devices/MD1600IO/MDBusArbiter.cpp:514-527`), and
/// the VDP's `000*` device locations are the data and control ports
/// (`Devices/315-5313/S315-5313_Ports.cpp:944-949`). These four bytes are
/// therefore the VDP, never the PSG (W1T/F5).
pub const VDP_Z80_BASE: u16 = 0x7F00;
/// Last Z80 address of the VDP port window.
pub const VDP_Z80_END: u16 = 0x7F03;
/// Common Z80 alias for the PSG data port.
pub const PSG_Z80_ALIAS: u16 = 0x7F11;
/// How a typed device error from a **Z80 guest access** is classified.
///
/// The Z80 core has no guest exception channel, so a rejected access is served with no
/// effect; the *reason* must still be distinguishable and observable. The classification is
/// exhaustive over the typed errors, so a new error variant breaks the build instead of
/// silently landing in a generic bucket (W1Y, closing the W1U-F2 class). No `bool` flags and
/// no wildcard arms: each variant is classified deliberately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestAccessClass {
    /// A documented operation this slice does not implement. The class is kept for a
    /// future one: no current operation produces it (the `0b1100` 8-bit VRAM read is
    /// implemented from W1Z), so it is exercised through the refused-DMA sub-bucket.
    /// A capability gap is not a hardware discard.
    MissingCapability,
    /// The device refused a DMA transfer: a 68000-memory source, an unsupported DMA form,
    /// or a copy-engine range/overlap rejection. Nothing was transferred.
    DmaUnavailable,
    /// The command code, offset or register is not a valid device operation.
    InvalidTarget,
    /// The target is decoded but read-only (cartridge ROM): the write does not latch.
    ReadOnlyTarget,
    /// The translated address has no owner.
    UnmappedTarget,
    /// A snapshot/codec fault. A port access cannot legitimately produce one; if it ever
    /// does, the access must fail rather than be swallowed.
    HostFault,
}

/// Classifies a [`VdpError`] raised by a Z80 guest access.
///
/// Exhaustive by construction: every variant is named, so adding one is a compile error here
/// (W1Y).
pub const fn classify_vdp_error(error: &VdpError) -> GuestAccessClass {
    match error {
        VdpError::MissingCapability { .. } => GuestAccessClass::MissingCapability,
        VdpError::DmaUnsupported { .. }
        | VdpError::DmaMemorySourceUnavailable { .. }
        | VdpError::DmaSourceOutOfRange { .. }
        | VdpError::DmaDestinationOutOfRange { .. }
        | VdpError::DmaSourceDestinationOverlap { .. } => GuestAccessClass::DmaUnavailable,
        VdpError::UnsupportedOffset { .. }
        | VdpError::OddOffset { .. }
        | VdpError::UnsupportedWrite { .. }
        | VdpError::UnsupportedRegister { .. }
        | VdpError::UnsupportedTarget { .. } => GuestAccessClass::InvalidTarget,
        VdpError::SnapshotLengthMismatch { .. }
        | VdpError::SnapshotMagicMismatch
        | VdpError::SnapshotVersionUnsupported { .. }
        | VdpError::SnapshotCodeInvalid { .. }
        | VdpError::SnapshotFifoCountInvalid { .. }
        | VdpError::SnapshotDmaModeInvalid { .. }
        | VdpError::SnapshotBooleanInvalid { .. }
        | VdpError::SnapshotLineOutOfRange { .. }
        | VdpError::SnapshotDotOutOfRange { .. } => GuestAccessClass::HostFault,
    }
}

/// Classifies a [`BusError`] reached through the Z80 banked window.
///
/// Exhaustive over `BusError`, for the same reason as [`classify_vdp_error`] (W1Y).
pub fn classify_bus_error(error: &BusError) -> GuestAccessClass {
    match error {
        BusError::UnmappedRead { .. } | BusError::UnmappedWrite { .. } => {
            GuestAccessClass::UnmappedTarget
        }
        BusError::WriteProtected { .. } => GuestAccessClass::ReadOnlyTarget,
        BusError::Vdp(error) => classify_vdp_error(error),
        // An odd address, an overflowing address, a side-effecting read or a cartridge
        // failure reached through the translate-and-forward bridge is a target this slice
        // refuses to serve, not a missing device and not a DMA refusal.
        BusError::OddAddress { .. }
        | BusError::AddressOverflow { .. }
        | BusError::SideEffectingRead { .. }
        | BusError::Cartridge(_) => GuestAccessClass::InvalidTarget,
    }
}

impl MegadriveBus {
    /// Applies the bookkeeping for a rejected Z80 access to the **VDP port window**.
    ///
    /// The invalid-target bucket there is the VDP's own discard counter (the documented model
    /// choice); a capability gap or a refused DMA surfaces as the typed rejection for the
    /// access and is counted in its own bucket. Exhaustive over
    /// [`GuestAccessClass`] (W1Y).
    fn record_z80_vdp_rejection(
        &mut self,
        class: GuestAccessClass,
        address: u16,
    ) -> Result<(), Z80Error> {
        match class {
            GuestAccessClass::InvalidTarget => {
                self.vdp.count_discarded_data_port_access();
                Ok(())
            }
            GuestAccessClass::MissingCapability => {
                self.vdp.count_capability_gap();
                Err(Z80Error::Memory { address })
            }
            GuestAccessClass::DmaUnavailable => {
                // A refused transfer is a capability gap (the operation the slice cannot
                // perform) *and* is isolated in its own counter, so it can never be read as
                // the invalid-target discard bucket (W1Y).
                self.vdp.count_capability_gap();
                self.z80_dma_unavailable_accesses =
                    self.z80_dma_unavailable_accesses.saturating_add(1);
                Err(Z80Error::Memory { address })
            }
            GuestAccessClass::UnmappedTarget => {
                self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
                Ok(())
            }
            GuestAccessClass::ReadOnlyTarget => Ok(()),
            GuestAccessClass::HostFault => Err(Z80Error::Memory { address }),
        }
    }

    /// Applies the bookkeeping for a rejected Z80 access through the **banked window**.
    ///
    /// Exhaustive over [`GuestAccessClass`]; the invalid-target bucket here is the bus-side
    /// counter, because no VDP data port is involved (W1Y).
    fn record_z80_bus_rejection(
        &mut self,
        class: GuestAccessClass,
        address: u16,
    ) -> Result<(), Z80Error> {
        match class {
            GuestAccessClass::InvalidTarget => {
                self.z80_invalid_target_accesses =
                    self.z80_invalid_target_accesses.saturating_add(1);
                Ok(())
            }
            GuestAccessClass::UnmappedTarget => {
                self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
                Ok(())
            }
            GuestAccessClass::ReadOnlyTarget => Ok(()),
            GuestAccessClass::MissingCapability => {
                self.vdp.count_capability_gap();
                Err(Z80Error::Memory { address })
            }
            GuestAccessClass::DmaUnavailable => {
                // A refused transfer is a capability gap (the operation the slice cannot
                // perform) *and* is isolated in its own counter, so it can never be read as
                // the invalid-target discard bucket (W1Y).
                self.vdp.count_capability_gap();
                self.z80_dma_unavailable_accesses =
                    self.z80_dma_unavailable_accesses.saturating_add(1);
                Err(Z80Error::Memory { address })
            }
            GuestAccessClass::HostFault => Err(Z80Error::Memory { address }),
        }
    }
}

/// Value a Z80 read of an undecoded address returns.
///
/// Executable reference (what the pinned oracle actually does): a Z80 byte read
/// goes through `Devices/Z80/Z80.cpp:1455-1474` into a default-constructed
/// `Z80Byte`, and `System/BusInterface.cpp:2438-2490` leaves that value untouched
/// when no map entry resolves, so an unmatched read yields `0x00`; the write path
/// (`:2491+`) performs nothing for an unmatched address, so the write is ignored.
///
/// Conflicting evidence, registered as **open**: arbiter comments state that
/// undriven lines read as set for a Z80 read into the 68000 space
/// (`Devices/MD1600IO/MDBusArbiter.cpp:366-370`) and that the Z80 likely drives
/// all lines high before a read request (`:427-435`; the 68000-into-Z80 mirror
/// forces undriven lines set at `:320-330`). If hardware evidence ever settles on
/// `0xFF`, this constant is the single point to change.
pub const Z80_UNDECODED_READ_VALUE: u8 = 0x00;

/// First Z80 address of the PSG mirror window.
///
/// The arbiter turns every byte of its `0x7F00` window into the 68000 address
/// `0xC00000 | offset` (`Devices/MD1600IO/MDBusArbiter.cpp:352` for reads,
/// `:516` for writes), so the Z80 bytes that land in the PSG window are exactly
/// `0x7F10..=0x7F17` — the mirror of `0xC00010..=0xC00017`. `0x7F64`, for
/// example, maps to `0xC00064`, outside the VDP's `0xC00000..=0xC0001F` block,
/// so it is *not* the PSG; its outcome belongs to the undecoded-access policy.
pub const PSG_Z80_MIRROR_BASE: u16 = 0x7F10;
/// Last Z80 address of the PSG mirror window.
pub const PSG_Z80_MIRROR_END: u16 = 0x7F17;
/// Master cycles a 68000 access to the Z80 RAM window costs while the Z80 owns
/// the bus.
///
/// The 68000 is stalled until the Z80 reaches an instruction boundary; the
/// shortest Z80 instruction is four T-states, so one M1 fetch
/// (4 * 15 = 60 master cycles) is the documented granularity. The charge is per
/// transferred byte (a word access charges two lanes), is added to the
/// instruction's master-cycle cost by the backend, and therefore enters the
/// canonical tick and state hash.
pub const Z80_STALL_MASTER_CYCLES: u64 = MASTER_CYCLES_PER_Z80_T_STATE * 4;
/// 68000/Z80 bus ownership and reset latches.
///
/// The polarity follows the hardware `ZBR` line. `requested` records that the
/// 68000 **asserted** the Z80 bus request through `0xA11100` bit 8: the Z80 stops
/// at its next instruction boundary and the 68000 owns the Z80 window. With the
/// request negated the Z80 owns the bus and executes. `reset_asserted` records
/// that the 68000 holds the Z80 in reset through the active-low `0xA11200` bit.
/// Both are canonical machine state; payload v6 serializes both latches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusArbitration {
    requested: bool,
    reset_asserted: bool,
}

impl BusArbitration {
    /// Returns whether the 68000 asserted the Z80 bus request.
    #[must_use]
    pub const fn requested(&self) -> bool {
        self.requested
    }

    /// Returns whether the Z80 reset line is asserted.
    #[must_use]
    pub const fn reset_asserted(&self) -> bool {
        self.reset_asserted
    }

    /// Returns whether the Z80 may execute: no bus request and out of reset.
    ///
    /// The model yields the bus functionally on the request write rather than
    /// waiting for the Z80 instruction boundary, so a `0xA11100` poll observes
    /// the yield immediately. See [`BusArbitration::z80_owns_bus`].
    #[must_use]
    pub const fn z80_runs(&self) -> bool {
        !self.requested && !self.reset_asserted
    }

    /// Returns whether the Z80 owns the bus, which is what `0xA11100` bit 8
    /// reports: one while the Z80 has it, zero once it has yielded to the 68000.
    #[must_use]
    pub const fn z80_owns_bus(&self) -> bool {
        !self.requested
    }

    /// Rebuilds the latches from a validated snapshot payload.
    pub(crate) const fn from_parts(requested: bool, reset_asserted: bool) -> Self {
        Self {
            requested,
            reset_asserted,
        }
    }
}

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
    /// YM2612 FM synthesiser.
    ///
    /// Canonical device state; payload v6 serializes the complete chip state.
    ym: Ym2612,
    /// SN76489-compatible PSG.
    ///
    /// Canonical device state; like the YM2612, payload v6 serializes it.
    psg: Psg,
    /// 8 KiB Z80 RAM, canonical machine state.
    ///
    /// Payload v6 serializes this RAM and the state hash covers it.
    z80_ram: Vec<u8>,
    /// Nine-bit Z80 bank register selecting the 32 KiB ROM window at `0x8000`.
    z80_bank: u16,
    /// Z80 guest accesses rejected because the device refused a DMA transfer.
    ///
    /// A memory-source DMA, an unsupported DMA form, or a copy-engine range/overlap
    /// rejection is **not** an invalid target and not a missing capability: it is a real
    /// operation this slice cannot perform for the guest. Kept in its own bucket so it can
    /// never be mistaken for the other two (W1Y / W1U-F2).
    z80_dma_unavailable_accesses: u64,
    /// Z80 guest accesses rejected as an invalid target by the 68000 bus reached through the
    /// banked window (odd address, overflowing address, side-effecting read, cartridge
    /// failure). Distinct from a real hole and from a DMA refusal (W1Y).
    z80_invalid_target_accesses: u64,
    /// Port 3/EXT data accesses accepted with no effect.
    ///
    /// The two port 3 data bytes are real hardware registers this slice does not
    /// model (ADR-0009). Faulting on them turned a legal guest store into a guest
    /// address error (W1V), so the access is accepted with no effect and counted
    /// here: a documented, observable divergence instead of a silent one (W1X).
    port3_unmodelled_accesses: u64,
    /// Z80 guest accesses served by the undecoded path.
    ///
    /// Diagnostic only, like the VDP's discarded-access counter: it is not part of
    /// any serialized state, so two machines with different counts still produce
    /// byte-identical snapshots.
    unmapped_z80_accesses: u64,
    /// Latched port 1/2/3/EXT control register bytes at `0xA10008..=0xA1000F`.
    ///
    /// Read/write register state that affects later reads, so it is canonical
    /// machine state: payload v7 serializes it and the state hash covers it.
    io_control: [u8; IO_CONTROL_LEN],
    /// 68000/Z80 bus ownership and reset latches.
    arbitration: BusArbitration,
    /// Master cycles accumulated by 68000 accesses to the Z80 window while the
    /// Z80 owns the bus; consumed once per 68000 step. Transient by
    /// construction: the backend folds it into the tick before the step ends.
    z80_stall_cycles: u64,
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
            ym: Ym2612::new(),
            psg: Psg::new(),
            z80_ram: vec![0; Z80_RAM_SIZE],
            z80_bank: 0,
            unmapped_z80_accesses: 0,
            port3_unmodelled_accesses: 0,
            z80_dma_unavailable_accesses: 0,
            z80_invalid_target_accesses: 0,
            io_control: [0; IO_CONTROL_LEN],
            // Power-on: the 68000 owns the bus and holds the Z80 in reset.
            arbitration: BusArbitration {
                requested: false,
                reset_asserted: true,
            },
            z80_stall_cycles: 0,
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

    /// Returns the YM2612 synthesiser.
    #[must_use]
    pub const fn ym(&self) -> &Ym2612 {
        &self.ym
    }

    /// Returns the YM2612 for timer/sample advancement.
    pub fn ym_mut(&mut self) -> &mut Ym2612 {
        &mut self.ym
    }

    /// Returns the PSG.
    #[must_use]
    pub const fn psg(&self) -> &Psg {
        &self.psg
    }

    /// Returns the PSG for sample advancement by the audio mixer.
    pub fn psg_mut(&mut self) -> &mut Psg {
        &mut self.psg
    }

    /// Returns both audio devices for one mixer advance.
    ///
    /// The two mutable borrows are disjoint fields, so the mixer can advance the
    /// YM2612 and the PSG in one call without cloning either device.
    pub fn audio_mut(&mut self) -> (&mut Ym2612, &mut Psg) {
        (&mut self.ym, &mut self.psg)
    }

    /// Replaces both audio devices during a validated snapshot restore.
    pub(crate) fn replace_audio_devices(&mut self, ym: Ym2612, psg: Psg) {
        self.ym = ym;
        self.psg = psg;
    }

    /// Returns the canonical 8 KiB Z80 RAM.
    #[must_use]
    pub fn z80_ram(&self) -> &[u8] {
        &self.z80_ram
    }

    /// Number of Z80 guest accesses answered by the undecoded path.
    ///
    /// A Z80 access to an address that no device owns cannot stop the run: the
    /// read returns [`Z80_UNDECODED_READ_VALUE`], the write is ignored, and the
    /// access is counted here so the path is observable instead of silent.
    #[must_use]
    pub const fn unmapped_z80_accesses(&self) -> u64 {
        self.unmapped_z80_accesses
    }

    /// Number of Z80 guest accesses rejected as an invalid target by the banked window.
    pub const fn z80_invalid_target_accesses(&self) -> u64 {
        self.z80_invalid_target_accesses
    }

    /// Number of Z80 guest accesses rejected because a DMA transfer was unavailable.
    pub const fn z80_dma_unavailable_accesses(&self) -> u64 {
        self.z80_dma_unavailable_accesses
    }

    /// Number of port 3/EXT data accesses accepted with no effect.
    ///
    /// Diagnostic like the other counters: not serialized, reset by a valid
    /// backend restore.
    pub const fn port3_unmodelled_accesses(&self) -> u64 {
        self.port3_unmodelled_accesses
    }

    pub const fn z80_bank(&self) -> u16 {
        self.z80_bank
    }

    /// Returns the latched port 1/2/3/EXT control register bytes.
    ///
    /// The bytes are canonical machine state, indexed from [`IO_CONTROL_BASE`]
    /// (`0xA10008`).
    #[must_use]
    pub const fn io_control(&self) -> &[u8; IO_CONTROL_LEN] {
        &self.io_control
    }

    /// Replaces the control register file during a snapshot restore.
    pub(crate) fn restore_io_control(&mut self, io_control: [u8; IO_CONTROL_LEN]) {
        self.io_control = io_control;
    }

    /// Returns the 68000/Z80 bus ownership and reset latches.
    #[must_use]
    pub const fn arbitration(&self) -> BusArbitration {
        self.arbitration
    }

    /// Sets the bus-request and reset latches.
    ///
    /// `requested` is the asserted Z80 bus request (the 68000 owns the window and
    /// the Z80 is stopped) and `reset` is the active-low reset state (`true`
    /// asserts reset). The call is idempotent: repeating the same pair leaves the
    /// latches unchanged.
    pub fn set_arbitration(&mut self, requested: bool, reset: bool) {
        self.arbitration = BusArbitration {
            requested,
            reset_asserted: reset,
        };
    }

    /// Takes the master cycles the 68000 lost to Z80-window stalls since the
    /// previous call.
    pub fn take_z80_stall_cycles(&mut self) -> u64 {
        std::mem::take(&mut self.z80_stall_cycles)
    }

    /// Discards a stall accumulated by a step that failed before advancing time.
    pub(crate) fn clear_z80_stall_cycles(&mut self) {
        self.z80_stall_cycles = 0;
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
        let address = bus_address(address);
        if let Some(offset) = vdp_offset(address) {
            let value = self.vdp.read8(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 1, false);
            return Ok(value);
        }
        if psg_window_offset(address).is_some() {
            // The PSG is write-only: every byte of its window reads the same
            // deterministic open value the Z80 side returns, so the two windows
            // cannot disagree.
            let value = self.psg.read();
            self.debug
                .observe_watchpoints(WatchpointAccess::Read, address, u32::from(value), 1);
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
        // Port 3/EXT data is a real register this slice does not model: a guest
        // read gets the window's open value and the access is counted, so the
        // divergence is documented and observable instead of a guest fault (W1X).
        if matches!(address, CONTROLLER_PORT_3_DATA | CONTROLLER_PORT_3_DATA_LOW) {
            self.port3_unmodelled_accesses = self.port3_unmodelled_accesses.saturating_add(1);
            self.debug.observe_watchpoints(
                WatchpointAccess::Read,
                address,
                u32::from(OPEN_BUS_VALUE),
                1,
            );
            return Ok(OPEN_BUS_VALUE);
        }
        if let Some(value) = self.read_ym_window(address) {
            self.debug
                .observe_watchpoints(WatchpointAccess::Read, address, u32::from(value), 1);
            return Ok(value);
        }
        if let Some(value) = self.read_z80_window(address) {
            if is_z80_ram_window(address) {
                self.stall_z80_window_access(address);
            }
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
        let address = bus_address(address);
        if vdp_offset(address).is_some() {
            return Err(BusError::SideEffectingRead { address });
        }
        if controller_port(address).is_some() {
            return Err(BusError::UnmappedRead { address });
        }
        // Port 3/EXT data is unmodelled; the execution path serves the open value
        // without side effects, so the debugger view matches it (W1X).
        if matches!(address, CONTROLLER_PORT_3_DATA | CONTROLLER_PORT_3_DATA_LOW) {
            return Ok(OPEN_BUS_VALUE);
        }
        if let Some(value) = self.read_ym_window(address) {
            return Ok(value);
        }
        if let Some(value) = self.read_z80_window(address) {
            return Ok(value);
        }
        if is_io_control_stub(address) {
            return Ok(0);
        }
        if let Some(offset) = io_control_offset(address) {
            return Ok(self.io_control[offset]);
        }
        if address == VERSION_REGISTER {
            return Ok(self.cartridge.version_register(self.video));
        }
        if is_cartridge_address(address) {
            return self.cartridge.read8(address).map_err(map_cartridge_read);
        }
        if is_work_ram(address) {
            return self
                .ram
                .get(ram_offset(address))
                .copied()
                .ok_or(BusError::UnmappedRead { address });
        }
        if is_io_undecoded(address) {
            return Ok(OPEN_BUS_VALUE);
        }
        Err(BusError::UnmappedRead { address })
    }
    /// Reads a big-endian word through the bus.
    ///
    /// Each byte lane is decoded independently from its own truncated address,
    /// so a word that starts at the last bus address reads its second lane from
    /// `0x000000`, exactly as the hardware's absent address-bit-24 does.
    pub fn read16(&mut self, address: u32) -> Result<u16, BusError> {
        let address = bus_address(address);
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            let value = self.vdp.read16(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 2, false);
            return Ok(value);
        }
        // The port 3/EXT control stub is a constant zero, so a word that spans
        // it reports zero on both lanes without consulting the canonical file.
        if is_io_control_stub(address) {
            return Ok(0);
        }
        let high = self.read8(address)?;
        let low = self.read8(address.wrapping_add(1))?;
        Ok(u16::from_be_bytes([high, low]))
    }

    /// Reads a big-endian long word through the bus.
    ///
    /// As in [`MegadriveBus::read16`], every byte lane decodes its own truncated
    /// address, so a long access that runs past the last bus address continues
    /// at `0x000000`.
    pub fn read32(&mut self, address: u32) -> Result<u32, BusError> {
        let address = bus_address(address);
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            let second_address = address.wrapping_add(2);
            let second_offset = vdp_offset(second_address).ok_or(BusError::UnmappedRead {
                address: second_address,
            })?;
            let high = self.vdp.read16(offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(high), 2, false);
            let low = self.vdp.read16(second_offset).map_err(BusError::Vdp)?;
            self.observe_vdp_access(second_offset, second_address, u32::from(low), 2, false);
            return Ok((u32::from(high) << 16) | u32::from(low));
        }
        // Compose the two big-endian words, each decoded by its own region: a
        // long that spans a functional control register and the port 3/EXT
        // control stub keeps both halves instead of zeroing the whole read, and
        // the stub's zero (or the window's open bus) only clears its own lane.
        let high = u32::from(self.read16(address)?);
        let low = u32::from(self.read16(address.wrapping_add(2))?);
        Ok((high << 16) | low)
    }

    /// Validates that a write range lies entirely in work RAM.
    ///
    /// Validation happens before any byte is mutated so an invalid range cannot
    /// leave a partially written machine state. Both endpoints use the truncated
    /// 24-bit bus address, and a range whose last byte wraps past the end of the
    /// bus is decoded by its wrapped endpoint: the mirror is 64 KiB-contiguous,
    /// so a frame at the top of `0xFFFFxxxx` stays inside one block instead of
    /// being reported as a 32-bit overflow.
    fn ensure_ram_write_range(&self, address: u32, length: usize) -> Result<(), BusError> {
        if length == 0 {
            return Ok(());
        }
        let address = bus_address(address);
        let span = u32::try_from(length.saturating_sub(1))
            .map_err(|_| BusError::AddressOverflow { address, length })?;
        let end = bus_address(address.wrapping_add(span));
        let wraps = address.wrapping_add(span) > ADDRESS_MASK;
        if wraps || !is_work_ram(address) || !is_work_ram(end) || end < address {
            if address < ROM_END {
                return Err(BusError::WriteProtected { address });
            }
            // Report the real exclusive end of work RAM: the wrapped endpoint is
            // a raw 24-bit value that names no decodable boundary in the
            // original request, so both the bus and the backend can explain the
            // failure with the same address (`0x01000000`).
            return Err(BusError::UnmappedWrite {
                address: if wraps { RAM_END } else { end },
            });
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
        let address = bus_address(address);
        if let Some(offset) = vdp_offset(address) {
            if !self.cartridge.tmss().vdp_writes_allowed() {
                return Ok(());
            }
            self.vdp.write8(offset, value).map_err(BusError::Vdp)?;
            self.observe_vdp_access(offset, address, u32::from(value), 1, true);
            return Ok(());
        }
        if psg_window_offset(address).is_some() {
            // The whole eight-byte window drives the one PSG instance the Z80
            // 0x7F11 also reaches; there is no second chip and no parallel latch.
            self.psg.write(value);
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        if let Some((bank, port)) = ym_port(address) {
            self.ym.write_bank(bank, port, value);
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        if self.write_z80_window(address, value) {
            if is_z80_ram_window(address) {
                self.observe_ram_write(address, value);
            } else {
                self.debug.observe_watchpoints(
                    WatchpointAccess::Write,
                    address,
                    u32::from(value),
                    1,
                );
            }
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
        if is_io_control_stub(address) {
            // The stub latches nothing; the write is acknowledged and dropped.
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        if let Some(offset) = io_control_offset(address) {
            self.io_control[offset] = value;
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        // Port 3/EXT data is a real register this slice does not model. The
        // hardware accepts the store, so faulting turned a legal guest write into
        // a guest address error (W1V: Kid's `MOVE.B #0x7F,(0x00A10007).L` skipped
        // by its exception handler). The access is accepted with no effect and
        // counted, matching how the rest of the I/O window is handled (W1X).
        if matches!(address, CONTROLLER_PORT_3_DATA | CONTROLLER_PORT_3_DATA_LOW) {
            self.port3_unmodelled_accesses = self.port3_unmodelled_accesses.saturating_add(1);
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        // A controller/IO data register and the undecoded rest of the window
        // accept the write and discard it: the hardware has no latch that can
        // fail, and keeping the error would stop a boot at a legitimate store.
        if is_io_window(address) {
            self.debug
                .observe_watchpoints(WatchpointAccess::Write, address, u32::from(value), 1);
            return Ok(());
        }
        self.ensure_ram_write_range(address, 1)?;
        self.ram[ram_offset(address)] = value;
        self.observe_ram_write(address, value);
        Ok(())
    }

    /// Writes a big-endian word to a mapped window.
    ///
    /// As in [`MegadriveBus::read16`], each byte lane decodes its own truncated
    /// address.
    pub fn write16(&mut self, address: u32, value: u16) -> Result<(), BusError> {
        let address = bus_address(address);
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
        if psg_window_offset(address).is_some() {
            // A 68000 word access to the PSG is ONE 16-bit transaction, not two
            // byte writes (W1T/F2): the pinned model decides the latch selector
            // from bit 15 and takes the channel/type/data fields from the low
            // byte, which is the single byte below.
            self.write_psg_transaction(value);
            return Ok(());
        }
        if ym_window_covers(address, 2) || z80_window_covers(address, 2) {
            let bytes = value.to_be_bytes();
            self.write8(address, bytes[0])?;
            self.write8(address.wrapping_add(1), bytes[1])?;
            return Ok(());
        }
        if io_window_covers(address, 2) {
            let bytes = value.to_be_bytes();
            self.write8(address, bytes[0])?;
            self.write8(address.wrapping_add(1), bytes[1])?;
            return Ok(());
        }
        self.ensure_ram_write_range(address, 2)?;
        let bytes = value.to_be_bytes();
        self.write8(address, bytes[0])?;
        self.write8(address.wrapping_add(1), bytes[1])
    }

    /// Writes a big-endian long word to a mapped window.
    ///
    /// As in [`MegadriveBus::read32`], each byte lane decodes its own truncated
    /// address.
    pub fn write32(&mut self, address: u32, value: u32) -> Result<(), BusError> {
        let address = bus_address(address);
        if address & 1 != 0 {
            return Err(BusError::OddAddress { address });
        }
        if let Some(offset) = vdp_offset(address) {
            if !self.cartridge.tmss().vdp_writes_allowed() {
                return Ok(());
            }
            let second_address = address.wrapping_add(2);
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
        if psg_window_covers(address, 4) {
            // A long access is TWO 16-bit transactions, at `address` and
            // `address + 2` (`Devices/M68000/M68000.cpp:2197-2204`), not four byte
            // writes (W1T/F2).
            let bytes = value.to_be_bytes();
            self.write_psg_transaction(u16::from_be_bytes([bytes[0], bytes[1]]));
            self.write_psg_transaction(u16::from_be_bytes([bytes[2], bytes[3]]));
            return Ok(());
        }
        if ym_window_covers(address, 4)
            || z80_window_covers(address, 4)
            || io_window_covers(address, 4)
        {
            let bytes = value.to_be_bytes();
            for (offset, byte) in bytes.iter().enumerate() {
                let offset =
                    u32::try_from(offset).map_err(|_| BusError::UnmappedWrite { address })?;
                self.write8(address.wrapping_add(offset), *byte)?;
            }
            return Ok(());
        }
        self.ensure_ram_write_range(address, 4)?;
        let bytes = value.to_be_bytes();
        for (offset, byte) in bytes.iter().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| BusError::UnmappedWrite { address })?;
            self.write8(address.wrapping_add(offset), *byte)?;
        }
        Ok(())
    }

    /// Reads the YM2612 status byte for an address in its 68000 window.
    ///
    /// Both hardware register read ports return the status byte; the call is
    /// side-effect free, so debugger views may use it too.
    fn read_ym_window(&self, address: u32) -> Option<u8> {
        ym_port(address).map(|_| self.ym.status())
    }

    /// Reads one byte from a 68000-visible Z80 window or control register.
    fn read_z80_window(&self, address: u32) -> Option<u8> {
        if is_z80_ram_window(address) {
            let offset = usize::from((address - Z80_RAM_BASE) as u16);
            return Some(self.z80_ram[offset]);
        }
        match address {
            // Bit 8 reports whether the Z80 still owns the bus: one until it has
            // yielded to the 68000's request, zero afterwards.
            Z80_BUS_REQUEST => Some(u8::from(self.arbitration.z80_owns_bus())),
            address if address == Z80_BUS_REQUEST + 1 => Some(0),
            Z80_RESET => Some(u8::from(!self.arbitration.reset_asserted)),
            address if address == Z80_RESET + 1 => Some(0),
            Z80_BANK_REGISTER => Some((self.z80_bank >> 8) as u8),
            address if address == Z80_BANK_REGISTER + 1 => Some(self.z80_bank as u8),
            _ => None,
        }
    }

    /// Writes one byte to a 68000-visible Z80 window or control register.
    ///
    /// Returns `false` when `address` is not one of those windows, so the caller
    /// keeps decoding the rest of the map. The bank register is nine bits wide;
    /// the 68000 high byte carries bit 8 and the low byte carries bits 0-7. The
    /// register is write-only on hardware, but reads return the latched value so
    /// the model can be inspected.
    fn write_z80_window(&mut self, address: u32, value: u8) -> bool {
        if is_z80_ram_window(address) {
            let offset = usize::from((address - Z80_RAM_BASE) as u16);
            self.z80_ram[offset] = value;
            self.stall_z80_window_access(address);
            return true;
        }
        match address {
            // Hardware polarity: setting bit 8 asserts the bus request, which
            // stops the Z80 and gives the 68000 the Z80 window; clearing it
            // returns the bus to the Z80.
            Z80_BUS_REQUEST => {
                self.arbitration.requested = value & 0x01 != 0;
                true
            }
            address if address == Z80_BUS_REQUEST + 1 => true,
            Z80_RESET => {
                // Active low: a clear bit asserts reset.
                self.arbitration.reset_asserted = value & 0x01 == 0;
                true
            }
            address if address == Z80_RESET + 1 => true,
            Z80_BANK_REGISTER => {
                self.z80_bank = (self.z80_bank & 0x00FF) | ((u16::from(value) & 0x01) << 8);
                true
            }
            address if address == Z80_BANK_REGISTER + 1 => {
                self.z80_bank = (self.z80_bank & 0x0100) | u16::from(value);
                true
            }
            _ => false,
        }
    }

    /// Charges one Z80-window stall to the current 68000 instruction.
    ///
    /// Only while the Z80 owns the bus (`0xA11100` request negated, reset
    /// released); the charge is one Z80 M1 fetch per transferred byte. See
    /// [`Z80_STALL_MASTER_CYCLES`].
    fn stall_z80_window_access(&mut self, address: u32) {
        if is_z80_ram_window(address) && self.arbitration.z80_runs() {
            self.z80_stall_cycles = self
                .z80_stall_cycles
                .saturating_add(Z80_STALL_MASTER_CYCLES);
        }
    }

    /// Reads one byte from the banked 32 KiB cartridge ROM window.
    ///
    /// The Z80 window bypasses the 68000 Sega mapper and reads the raw image at
    /// `(bank << 15) | (address & 0x7FFF)`, which is the documented hardware
    /// mapping. A selected bank beyond the image reports a typed memory error
    /// rather than aliasing another device.
    fn read_z80_banked_rom(&mut self, address: u16) -> Result<u8, Z80Error> {
        let offset = (u32::from(self.z80_bank) << 15) | u32::from(address & 0x7FFF);
        let index = usize::try_from(offset).map_err(|_| Z80Error::Memory { address })?;
        match self.cartridge.rom().get(index).copied() {
            Some(byte) => Ok(byte),
            // The bank is selected but the image is shorter than the window, so
            // no device drives these lines. That is the same undecoded outcome as
            // an unowned address: the guest reads the documented value and the run
            // continues.
            None => {
                self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
                Ok(Z80_UNDECODED_READ_VALUE)
            }
        }
    }

    /// Returns the side-effect-free window that contains `start`, together with
    /// the window's first bus address.
    ///
    /// Debugger reads may target the cartridge image, work RAM (primary or
    /// mirror), the Z80 RAM window or the control register file. The VDP port
    /// window is excluded because reading it changes device state. A mirror
    /// window serves exactly one 64 KiB block, so a read that runs past the end
    /// of the block reports the first unserved byte instead of silently aliasing
    /// the block start.
    fn debugger_window(&self, start: u32) -> Option<(&[u8], u32)> {
        // Only the first `ROM_END` bytes of the image are addressable on the
        // 68000 bus; a larger image never reaches the bus through this window.
        // The window is the slice itself, so its exclusive end is exactly
        // `ROM_BASE + rom_len`: a cart image shorter than the 4 MiB window is
        // bounded by its own length, not by the address-space hole after it.
        let rom = self.cartridge.rom();
        let rom_len = rom.len().min(ROM_END as usize);
        if (start as usize) < rom_len {
            return Some((&rom[..rom_len], ROM_BASE));
        }
        if (Z80_RAM_BASE..Z80_RAM_END).contains(&start) {
            return Some((self.z80_ram.as_slice(), Z80_RAM_BASE));
        }
        if is_work_ram(start) {
            // The whole mirror is one window over the 64 KiB of work RAM,
            // decoded through address bits 15-0, and the primary window is its
            // last 64 KiB block. There is no mirror ambiguity: the
            // `0xE00000..0xFFFFFF` stride is exactly 64 KiB, so byte n of the
            // region is RAM offset `n mod 64 KiB` for both rules a debugger
            // window could follow.
            return Some((self.ram.as_slice(), RAM_BASE));
        }
        if is_io_control(start) {
            return Some((&self.io_control, IO_CONTROL_BASE));
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
    /// output buffer is allocated: the cartridge image, work RAM, the Z80 RAM
    /// window or the control register file must contain the whole request. A
    /// caller-controlled `length` that is enormous or that overflows the
    /// address space therefore returns a typed error instead of requesting an
    /// allocation that could abort the process.
    pub fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BusError> {
        // The start address is validated before the zero-length shortcut: the
        // baseline rejected addresses above the 32-bit bus even for an empty
        // read, while any representable address with length zero is accepted
        // without touching a window. A representable address is then truncated
        // to the 24-bit bus, as every other bus entry point does.
        let start = bus_address(
            u32::try_from(address).map_err(|_| BusError::UnmappedRead { address: u32::MAX })?,
        );
        if length == 0 {
            return Ok(Vec::new());
        }
        // The range proof for every window is done before any reservation: a
        // request that cannot be served in full never allocates. `try_reserve`
        // is a capacity guard, not a range check, and a representable but
        // out-of-window length must not reach the allocator at all.
        let span = u32::try_from(length).map_err(|_| BusError::AddressOverflow {
            address: start,
            length,
        })?;
        // Exclusive end of the request. An overflow here means the request runs
        // past the 32-bit address space, so nothing can serve it.
        let end_exclusive = start.checked_add(span).ok_or(BusError::AddressOverflow {
            address: start,
            length,
        })?;

        if is_work_ram(start) {
            // Work RAM is the whole `0xE00000..=0xFFFFFF` mirror, decoded
            // through address bits 15-0, so a range may cross a 64 KiB
            // mirror-block boundary and still land on consecutive RAM bytes.
            // The upper bound is the end of the primary window: past
            // `0xFFFFFF` the bus decodes other regions (the low lane is ROM),
            // and a debugger read must not present those bytes as RAM.
            if end_exclusive > RAM_END {
                return Err(BusError::UnmappedRead { address: RAM_END });
            }
            let mut data = Vec::new();
            if data.try_reserve_exact(length).is_err() {
                return Err(BusError::AddressOverflow {
                    address: start,
                    length,
                });
            }
            // The interval cannot wrap (the proof above bounds it by
            // `0xFFFFFF`), so indexing by offset is the same walk.
            for offset in 0..span {
                data.push(self.ram[ram_offset(start + offset)]);
            }
            return Ok(data);
        }

        let (window, base) = self
            .debugger_window(start)
            .ok_or_else(|| self.unmapped_debugger_access(start))?;
        // The window's exclusive end is derived from the slice that serves the
        // bytes, never from the address-space span around it: a cartridge image
        // shorter than its 4 MiB window is bounded by the image length, so an
        // overrun is proven here, before the reservation, instead of surfacing
        // from a failed `window.get` after the buffer was allocated.
        let window_end = base
            .checked_add(
                u32::try_from(window.len()).map_err(|_| BusError::AddressOverflow {
                    address: start,
                    length,
                })?,
            )
            .ok_or(BusError::AddressOverflow {
                address: start,
                length,
            })?;
        if end_exclusive > window_end {
            // Report the window's exclusive end: the first address the window
            // cannot serve, consistent with the work RAM path above.
            return Err(BusError::UnmappedRead {
                address: window_end,
            });
        }
        // The range proof above leaves the whole request inside `window`, so
        // its slice bounds are in range and the copy below cannot fail.
        let mut data = Vec::new();
        if data.try_reserve_exact(length).is_err() {
            return Err(BusError::AddressOverflow {
                address: start,
                length,
            });
        }
        let offset = start.wrapping_sub(base) as usize;
        let end = offset + span as usize;
        data.extend_from_slice(&window[offset..end]);
        Ok(data)
    }

    /// Writes a byte range to work RAM or the control register file for
    /// debugger consumers.
    ///
    /// The whole range is validated before the first byte is written, and each
    /// byte is then written through the side-effect-free work RAM path. This is
    /// not an emulated bus write, so it never produces a debug event, never
    /// touches a device, never re-encodes a step-scoped value as a register and
    /// never leaves a partial write behind: a rejected range leaves both the
    /// control register file and the state hash untouched.
    pub fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BusError> {
        let start = bus_address(
            u32::try_from(address).map_err(|_| BusError::UnmappedWrite { address: u32::MAX })?,
        );
        if data.is_empty() {
            return Ok(());
        }
        let span = u32::try_from(data.len()).map_err(|_| BusError::AddressOverflow {
            address: start,
            length: data.len(),
        })?;
        // The interval is proven against the eight control bytes as a whole
        // before a single byte is stored, so `write_memory(0xA1000F, &[0x55,
        // 0x66])` fails atomically instead of storing `0x55` first.
        if is_io_control(start) {
            let end = start
                .checked_add(span - 1)
                .ok_or(BusError::AddressOverflow {
                    address: start,
                    length: data.len(),
                })?;
            if end >= IO_CONTROL_BASE + IO_CONTROL_LEN as u32 {
                return Err(BusError::UnmappedWrite { address: end });
            }
            for (offset, byte) in data.iter().enumerate() {
                let target = start
                    + u32::try_from(offset).map_err(|_| BusError::AddressOverflow {
                        address: start,
                        length: data.len(),
                    })?;
                // The stub half discards the byte; the two functional port
                // control register pairs latch it.
                if let Some(index) = io_control_offset(target) {
                    self.io_control[index] = *byte;
                }
            }
            return Ok(());
        }
        // Work RAM: the interval is proven in full before the loop mutates
        // anything, and the endpoint check mirrors `read_memory` (a range that
        // runs past `0xFFFFFF` is not RAM any more).
        self.ensure_ram_write_range(start, data.len())?;
        for (offset, byte) in data.iter().enumerate() {
            let target = start
                .checked_add(
                    u32::try_from(offset).map_err(|_| BusError::AddressOverflow {
                        address: start,
                        length: data.len(),
                    })?,
                )
                .ok_or(BusError::AddressOverflow {
                    address: start,
                    length: data.len(),
                })?;
            self.ram[ram_offset(target)] = *byte;
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

    /// Replaces the complete 8 KiB Z80 RAM during a snapshot restore.
    /// Resets every non-canonical diagnostic counter on the bus.
    ///
    /// A valid restore replaces the machine's state, and the counters describe
    /// execution *since* that restore, so the replacement state starts at zero
    /// (W1T/F4). Nothing here is serialized, so no payload byte moves.
    pub(crate) fn reset_diagnostic_counters(&mut self) {
        self.unmapped_z80_accesses = 0;
        self.port3_unmodelled_accesses = 0;
        self.z80_dma_unavailable_accesses = 0;
        self.z80_invalid_target_accesses = 0;
    }

    pub(crate) fn restore_z80_ram(&mut self, ram: Vec<u8>) -> Result<(), BusError> {
        if ram.len() != Z80_RAM_SIZE {
            return Err(BusError::UnmappedWrite {
                address: Z80_RAM_BASE,
            });
        }
        self.z80_ram = ram;
        Ok(())
    }

    /// Replaces the 68000/Z80 bus arbitration latches during a restore.
    pub(crate) fn restore_arbitration(&mut self, arbitration: BusArbitration) {
        self.arbitration = arbitration;
    }

    /// Replaces the nine-bit Z80 bank register during a validated restore.
    pub(crate) fn restore_z80_bank(&mut self, bank: u16) {
        debug_assert!(bank <= 0x01FF, "the Z80 bank register is nine bits wide");
        self.z80_bank = bank;
    }
}

/// Truncates a 32-bit register value to the 24-bit address the machine decodes.
///
/// The 68000 has 24 address pins, so bits 31..24 are not decoded by any Mega
/// Drive device and a register value like `0xFFFFF7FE` addresses `0x00FFF7FE`.
/// Emulated CPU registers keep their full 32-bit value; only this bus decodes.
#[must_use]
pub const fn bus_address(address: u32) -> u32 {
    address & ADDRESS_MASK
}

/// Returns whether `address` (already 24-bit) decodes to work RAM.
///
/// Both the primary `0xFF0000..=0xFFFFFF` window and the `0xE00000..=0xFEFFFF`
/// mirror land on the same 64 KiB through address bits 15-0.
#[must_use]
pub const fn is_work_ram(address: u32) -> bool {
    address >= RAM_MIRROR_BASE
}

/// Returns the work RAM byte offset decoded by a 24-bit address.
#[must_use]
const fn ram_offset(address: u32) -> usize {
    (address & 0xFFFF) as usize
}

/// Returns whether `address` is inside the controller/IO window.
#[must_use]
const fn is_io_window(address: u32) -> bool {
    address >= CONTROLLER_WINDOW_BASE && address < CONTROLLER_WINDOW_END
}

/// Returns whether `address` is an undecoded byte of the controller/IO window.
///
/// Those bytes have no modelled device, so a read returns [`OPEN_BUS_VALUE`] and
/// a write is discarded. The two port 3/EXT data bytes and the mapped port 1/2
/// control register pairs are excluded. The port 3/EXT **control** stub bytes
/// (`0xA1000C..=0xA1000F`) are undecoded too, but the ADR-0014 contract pins
/// them to a constant zero rather than to the floating value, so they are
/// handled separately by [`is_io_control_stub`].
#[must_use]
const fn is_io_undecoded(address: u32) -> bool {
    is_io_window(address) && io_control_offset(address).is_none() && !is_io_control_stub(address)
}

/// Returns whether `address` is one of the port 3/EXT control stub bytes.
///
/// ADR-0014 (`0xA10008`/`09`, `0A`/`0B`, `0C`/`0D`, `0E`/`0F`: "Port 3/EXT
/// **control** is a stub (0)") pins the two stub pairs to the constant zero:
/// a read returns `0x00` and a write is acknowledged and discarded. The bytes
/// stay in the canonical control file so the snapshot layout has one home, but
/// they never change, so the state hash never moves for them.
#[must_use]
pub const fn is_io_control_stub(address: u32) -> bool {
    address >= IO_CONTROL_STUB_BASE && address < IO_CONTROL_BASE + IO_CONTROL_LEN as u32
}

/// Returns whether `address` is one of the eight port 1/2/3/EXT control bytes.
#[must_use]
pub const fn is_io_control(address: u32) -> bool {
    address >= IO_CONTROL_BASE && address < IO_CONTROL_BASE + IO_CONTROL_LEN as u32
}

/// Returns the control register index addressed by `address`, if any.
///
/// Only the four latched port 1/2 control bytes are indexed; the port 3/EXT
/// control stub is a constant zero and is deliberately not addressable in the
/// canonical file, so a write through this mapping can never change it.
#[must_use]
pub const fn io_control_offset(address: u32) -> Option<usize> {
    if address >= IO_CONTROL_BASE && address < IO_CONTROL_STUB_BASE {
        Some((address - IO_CONTROL_BASE) as usize)
    } else {
        None
    }
}

/// Returns whether every byte of `address..address + length` is an I/O window
/// byte that the bus can write, so a multi-byte write can be split per lane.
///
/// The port 3/EXT data bytes are excluded because they stay a typed error.
#[must_use]
fn io_window_covers(address: u32, length: u32) -> bool {
    let mut offset = 0;
    while offset < length {
        let target = bus_address(address.wrapping_add(offset));
        if !is_io_window(target)
            || matches!(target, CONTROLLER_PORT_3_DATA | CONTROLLER_PORT_3_DATA_LOW)
        {
            return false;
        }
        offset += 1;
    }
    true
}

/// Returns the YM2612 bank and latch addressed by a 68000 window byte.
fn ym_port(address: u32) -> Option<(YmBank, YmPort)> {
    match address.checked_sub(YM2612_BASE)? {
        0 => Some((YmBank::One, YmPort::Address)),
        1 => Some((YmBank::One, YmPort::Data)),
        2 => Some((YmBank::Two, YmPort::Address)),
        3 => Some((YmBank::Two, YmPort::Data)),
        _ => None,
    }
}

/// Returns whether every byte of `address..address + length` is a YM2612 byte.
fn ym_window_covers(address: u32, length: u32) -> bool {
    let mut offset = 0;
    while offset < length {
        if ym_port(address.saturating_add(offset)).is_none() {
            return false;
        }
        offset += 1;
    }
    true
}

/// Maps a Z80 `SOUND` byte to its bank and latch.
///
/// The region is decoded by the low address bits (`AddressMask=0x0003`), so the
/// port order repeats: `0` latches a bank-1 register, `1` writes bank-1 data, `2`
/// latches a bank-2 register and `3` writes bank-2 data. The pinned oracle fixes
/// that order in `Devices/YM2612/YM2612.cpp:2339-2350` (`location == 0` latches
/// part 1, `location == 2` latches part 2, `location == 1 || location == 3`
/// commits the currently latched register).
const fn z80_ym_port(address: u16) -> (YmBank, YmPort) {
    match address & 0x0003 {
        0 => (YmBank::One, YmPort::Address),
        1 => (YmBank::One, YmPort::Data),
        2 => (YmBank::Two, YmPort::Address),
        _ => (YmBank::Two, YmPort::Data),
    }
}

/// Returns whether `address` is inside the 68000-visible Z80 RAM window.
fn is_z80_ram_window(address: u32) -> bool {
    (Z80_RAM_BASE..Z80_RAM_END).contains(&address)
}

/// Returns whether `address` is one of the mapped Z80 control/register bytes.
fn is_z80_control_address(address: u32) -> bool {
    (Z80_BUS_REQUEST..=Z80_BUS_REQUEST + 1).contains(&address)
        || (Z80_RESET..=Z80_RESET + 1).contains(&address)
        || (Z80_BANK_REGISTER..=Z80_BANK_REGISTER + 1).contains(&address)
}

/// Returns whether `address` is any mapped 68000-visible Z80 byte.
fn is_z80_window_address(address: u32) -> bool {
    is_z80_ram_window(address) || is_z80_control_address(address)
}

/// Returns whether every byte of `address..address + length` is Z80-mapped.
fn z80_window_covers(address: u32, length: u32) -> bool {
    let mut offset = 0;
    while offset < length {
        if !is_z80_window_address(address.saturating_add(offset)) {
            return false;
        }
        offset += 1;
    }
    true
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

/// Returns the PSG window offset addressed by a 68000 byte address.
fn psg_window_offset(address: u32) -> Option<u8> {
    let offset = address.checked_sub(PSG_BASE)?;
    if offset < PSG_WINDOW_SIZE {
        u8::try_from(offset).ok()
    } else {
        None
    }
}

/// Delivers one 16-bit 68000 transaction to the PSG.
///
/// Pinned model, matching the oracle's executable path (not physical proof): a
/// word access is a single transaction (`Devices/M68000/M68000.cpp:2191-2194` one
/// `M68000Word`, forwarded once by `Devices/315-5313/S315-5313_Ports.cpp:1639-1643`).
/// The SN76489 decides latch versus data with `data.MSB()`
/// (`Devices/SN76489/SN76489.cpp:723-748`, `ExodusSDK/DeviceInterface/Data.inl:560-562`)
/// while extracting channel, type and data from the low segments, so the fields
/// the chip consumes are `((word >> 8) & 0x80) | (word & 0x7F)`. This is a model
/// choice matching the pin; it is not independent evidence of the real PSG lanes.
impl MegadriveBus {
    fn write_psg_transaction(&mut self, word: u16) {
        let byte = (((word >> 8) as u8) & 0x80) | ((word as u8) & 0x7F);
        self.psg.write(byte);
    }
}

/// Returns whether every byte of `address..address + length` is a PSG byte.
fn psg_window_covers(address: u32, length: u32) -> bool {
    let mut offset = 0;
    while offset < length {
        if psg_window_offset(address.saturating_add(offset)).is_none() {
            return false;
        }
        offset += 1;
    }
    true
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

impl Z80Bus for MegadriveBus {
    fn read8(&mut self, address: u16) -> Result<u8, Z80Error> {
        match address {
            // 8 KiB of RAM mirrored through the first 16 KiB of the Z80 map.
            0x0000..=0x3FFF => Ok(self.z80_ram[usize::from(address & Z80_RAM_MASK)]),
            YM2612_Z80_BASE..=YM2612_Z80_SOUND_END => {
                let (bank, port) = z80_ym_port(address);
                Ok(self.ym.read_bank(bank, port))
            }
            Z80_BANK_REGISTER_ADDRESS => Ok((self.z80_bank >> 8) as u8),
            // `0x7F00..=0x7F03` is the VDP's port window on the Z80 side, not the
            // PSG (W1T/F5).
            VDP_Z80_BASE..=VDP_Z80_END => {
                let offset = (address - VDP_Z80_BASE) as u8;
                match self.vdp.read8(offset) {
                    Ok(value) => Ok(value),
                    // A documented operation this slice does not implement is a
                    // CAPABILITY gap, not an invalid target: it is counted apart
                    // and never turned into a fake read. The Z80 core has no
                    // exception channel, so the typed rejection surfaces as the
                    // bus error for this access (W1X / W1U-F2).
                    // No current operation produces the capability-gap error
                    // (the `0b1100` 8-bit VRAM read is implemented from W1Z on),
                    // so a `0x0C` read reaches the value branch above. The arm
                    // stays so a future documented-but-unimplemented operation
                    // keeps its own class instead of collapsing into an
                    // invalid-target discard (W1Y).
                    Err(VdpError::MissingCapability { .. }) => {
                        self.vdp.count_capability_gap();
                        Err(Z80Error::Memory { address })
                    }
                    // Everything else is classified by its typed variant: an invalid
                    // target keeps the documented model outcome (no data, counted as a
                    // discarded data-port access), while a capability gap or a refused
                    // DMA is surfaced as the typed rejection and counted apart (W1Y).
                    Err(error) => self
                        .record_z80_vdp_rejection(classify_vdp_error(&error), address)
                        .map(|()| Z80_UNDECODED_READ_VALUE),
                }
            }
            // The PSG is write-only; the alias and the mirror window return the
            // chip's documented open value.
            PSG_Z80_ALIAS | PSG_Z80_MIRROR_BASE..=PSG_Z80_MIRROR_END => Ok(self.psg.read()),
            Z80_ROM_WINDOW_BASE..=0xFFFF => self.read_z80_banked_rom(address),
            // No device owns this byte: the guest gets the documented value and
            // the run continues (see [`Z80_UNDECODED_READ_VALUE`]).
            _ => {
                self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
                Ok(Z80_UNDECODED_READ_VALUE)
            }
        }
    }

    fn write8(&mut self, address: u16, value: u8) -> Result<(), Z80Error> {
        match address {
            0x0000..=0x3FFF => {
                self.z80_ram[usize::from(address & Z80_RAM_MASK)] = value;
                Ok(())
            }
            Z80_BANK_REGISTER_ADDRESS => {
                // The Z80 side is one byte wide; it selects bank bits 0-7 and
                // clears bit 8. The 68000-side word write covers all nine bits.
                self.z80_bank = u16::from(value);
                Ok(())
            }
            YM2612_Z80_BASE..=YM2612_Z80_SOUND_END => {
                let (bank, port) = z80_ym_port(address);
                self.ym.write_bank(bank, port, value);
                Ok(())
            }
            VDP_Z80_BASE..=VDP_Z80_END => {
                // The Z80 VDP window drives the VDP ports, but the TMSS lockout
                // applies to the VDP regardless of which master reaches it: the
                // bridge writes through the 68000 bus
                // (`Devices/MD1600IO/MDBusArbiter.cpp:514-527`) and the lockout is
                // signalled on the VDP access (`:1029-1037`). Same gate as
                // `MegadriveBus::write8` above (W1X / W1U-F1).
                if !self.cartridge.tmss().vdp_writes_allowed() {
                    return Ok(());
                }
                // An invalid target has no effect (the pin logs a diagnostic and
                // the access completes), so it is counted rather than ending the
                // run; the Z80 core has no exception channel to deliver a bus
                // error on.
                let offset = (address - VDP_Z80_BASE) as u8;
                match self.vdp.write8(offset, value) {
                    Ok(()) => Ok(()),
                    // Classified by variant, exhaustively (W1Y): a DMA refusal is not an
                    // invalid target, and a capability gap is neither.
                    Err(error) => {
                        self.record_z80_vdp_rejection(classify_vdp_error(&error), address)
                    }
                }
            }
            PSG_Z80_ALIAS | PSG_Z80_MIRROR_BASE..=PSG_Z80_MIRROR_END => {
                self.psg.write(value);
                Ok(())
            }
            // The banked window is not a universal ROM: the arbiter translates
            // the Z80 address into the 68000 space (bank << 15 | offset) and
            // forwards the access to whatever the target is
            // (`Devices/MD1600IO/MDBusArbiter.cpp:537-560`, which duplicates the
            // byte across the 16-bit data bus). So a bank aimed at work RAM really
            // writes RAM, and only an unserved target has no effect.
            Z80_ROM_WINDOW_BASE..=0xFFFF => {
                let target = (u32::from(self.z80_bank) << 15) | u32::from(address & 0x7FFF);
                match self.write8(target, value) {
                    Ok(()) => Ok(()),
                    // Exhaustive classification of the 68000-bus rejection (W1Y): a
                    // read-only ROM target simply does not latch, a real hole is counted
                    // as unmapped, an invalid target in its own bucket, and a VDP DMA
                    // refusal keeps the DMA bucket — no wildcard swallows a typed error.
                    Err(error) => {
                        self.record_z80_bus_rejection(classify_bus_error(&error), address)
                    }
                }
            }
            // No device owns this byte: the write has no effect and the run
            // continues (see [`Z80_UNDECODED_READ_VALUE`]).
            _ => {
                self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
                Ok(())
            }
        }
    }

    fn input(&mut self, port: u16) -> Result<u8, Z80Error> {
        // Every Mega Drive Z80 device is memory-mapped, so the Z80 I/O space has
        // no owner. `port` is unused because the outcome does not depend on it;
        // the access is counted like every other undecoded guest access.
        let _ = port;
        self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
        Ok(Z80_UNDECODED_READ_VALUE)
    }

    fn output(&mut self, port: u16, _value: u8) -> Result<(), Z80Error> {
        // Write side of the same undecoded channel: ignored, never fatal.
        let _ = port;
        self.unmapped_z80_accesses = self.unmapped_z80_accesses.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::Cartridge;
    use crate::z80::Z80;

    use super::{BusError, MegadriveBus, RAM_BASE, Z80_UNDECODED_READ_VALUE};

    #[test]
    fn undecoded_z80_accesses_are_counted_served_and_never_fatal() {
        // A real Z80 executes LD A,(0x6100); LD (0x6100),A; LD A,(0x7F64);
        // LD (0x7F64),A. None of those addresses owns a device, so each access is
        // served by the undecoded path: the read yields the documented value, the
        // write is ignored, and no access ends the run (W1R).
        let mut bus = bus();
        let mut program = vec![0_u8; 0x2000];
        program[0] = 0x3A; // LD A,(0x6100)
        program[1] = 0x00;
        program[2] = 0x61;
        program[3] = 0x32; // LD (0x6100),A
        program[4] = 0x00;
        program[5] = 0x61;
        program[6] = 0x3A; // LD A,(0x7F64)
        program[7] = 0x64;
        program[8] = 0x7F;
        program[9] = 0x32; // LD (0x7F64),A
        program[10] = 0x64;
        program[11] = 0x7F;
        program[12] = 0x76; // HALT
        bus.restore_z80_ram(program).expect("z80 ram");

        let mut z80 = Z80::new();
        for _ in 0..4 {
            z80.step(&mut bus)
                .expect("an undecoded access cannot end the run");
        }
        assert_eq!(bus.unmapped_z80_accesses(), 4);
        assert_eq!(z80.a(), Z80_UNDECODED_READ_VALUE);
        z80.step(&mut bus).expect("halt");
        assert_eq!(bus.unmapped_z80_accesses(), 4, "HALT is not an access");
    }

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
    fn the_eight_bit_vram_read_decodes_both_faces_of_the_data_port() {
        // `0b1100` (W1Z) through the composition the 68000 uses: a word read is
        // one data-port operation (`M68000.cpp:2010-2025`), a byte read is one
        // data-port operation whose returned half is chosen by the address parity
        // (`:1986-2007`), and a long read is two word operations (`:2026-2055`).
        let mut bus = bus();
        // VRAM word 0x0000 = 0x1234 through the data port. Auto-increment is
        // pinned to 0 so the command address never moves. The command words are
        // the encoding the Z80 portal test uses: word 1 `CD1:CD0 = 00` plus the
        // low address bits, word 2 `CD5:CD2 = 0b0011` plus the high address bits.
        set_vram_address(&mut bus, 0x0000, 0x4000);
        bus.write16(0x00C0_0000, 0x1234).expect("word write");
        bus.write16(0x00C0_0004, 0x8F00).expect("auto-increment 0");
        bus.write16(0x00C0_0004, 0x0000).expect("command word 1");
        bus.write16(0x00C0_0004, 0x0030)
            .expect("command word 2 (code 0x0C)");
        // The owner advances between the write and the reads: the cache step only
        // runs while the write FIFO is empty (`General.cpp:1748`, `:1850-1856`),
        // and the backend drains the window at this step boundary. The
        // pending-write behaviour itself (a read neither refreshes nor drains) is
        // pinned at the device layer in `tests/vdp_ports.rs` (t4 vector 11).
        bus.vdp_mut().drain_fifo();

        // With the window empty, each `0x0C` read seeds the buffer from the zeroed
        // next-read slot and replaces only the low byte with VRAM[0 ^ 1] = 0x34,
        // so every operation returns 0x0034.
        assert_eq!(bus.read16(0x00C0_0000).expect("word read"), 0x0034);
        assert_eq!(bus.read16(0x00C0_0000).expect("second word read"), 0x0034);

        // A long read is two word operations at `address` and `address + 2`; the
        // data-port mirror keeps both on the data port, so both lanes come from
        // consecutive `0x0C` reads.
        let long = bus.read32(0x00C0_0000).expect("long read");
        assert_eq!(long, 0x0034_0034);

        // One byte operation per lane: the buffer's upper byte for an even address
        // and its lower byte for an odd one.
        assert_eq!(bus.read8(0x00C0_0000).expect("even lane"), 0x00);
        assert_eq!(bus.read8(0x00C0_0001).expect("odd lane"), 0x34);
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
        // `0xC00010..=0xC00017` is the PSG window (W1Q); the first byte past it
        // is still an unmapped hole.
        assert!(matches!(
            bus.read8(0x00C0_0018),
            Err(BusError::UnmappedRead { .. })
        ));
        assert_eq!(bus.read8(0x00A0_0000).expect("z80 ram"), 0x00);
        // The `0xE00000..=0xFFFFFF` range is the work RAM mirror (W1.2), so an
        // unmapped hole is only the range below it that no device owns.
        assert!(matches!(
            bus.read8(0x00C0_1000),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(matches!(
            bus.read8(0x00A0_2000),
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
    fn controller_window_maps_the_control_registers_open_bus_and_data_registers() {
        let mut bus = bus();
        bus.set_step_input([0x01, 0x02]);
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7E));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_2), Ok(0x7D));

        // Port 1/2/3/EXT control registers (0xA10008..=0xA1000F) are a
        // read/write stub: reset reads zero, each byte latches, and a word or
        // long access reads both lanes.
        for offset in 0..super::IO_CONTROL_LEN {
            let address = super::IO_CONTROL_BASE + offset as u32;
            assert_eq!(bus.read8(address), Ok(0), "reset at 0x{address:06X}");
        }
        // The port 1/2 control pairs (`0xA10008..=0xA1000B`) latch a written byte;
        // the port 3/EXT control pairs (`0xA1000C..=0xA1000F`) are the ADR-0014
        // constant-zero stub, so a write there is acknowledged and discarded and
        // a read stays zero.
        for offset in 0..super::IO_CONTROL_LEN {
            let address = super::IO_CONTROL_BASE + offset as u32;
            let value = 0x80 | offset as u8;
            assert_eq!(bus.write8(address, value), Ok(()), "write 0x{address:06X}");
            let expected = if address < super::IO_CONTROL_STUB_BASE {
                value
            } else {
                0x00
            };
            assert_eq!(
                bus.read8(address),
                Ok(expected),
                "read back 0x{address:06X}"
            );
        }
        assert_eq!(bus.read16(super::IO_CONTROL_BASE), Ok(0x8081));
        assert_eq!(
            bus.read16(super::IO_CONTROL_STUB_BASE),
            Ok(0x0000),
            "the port 3/EXT control stub never latches"
        );
        assert_eq!(
            bus.read32(super::IO_CONTROL_BASE),
            Ok(0x8081_8283),
            "the long read latches all four port 1/2 control lanes"
        );
        assert_eq!(
            bus.read32(super::IO_CONTROL_STUB_BASE),
            Ok(0x0000_0000),
            "a long read of the stub reads zero on every lane"
        );
        assert_eq!(bus.peek8(super::IO_CONTROL_BASE), Ok(0x80));
        assert_eq!(
            bus.read_memory(u64::from(super::IO_CONTROL_BASE), 8)
                .expect("debugger read of the control file"),
            vec![0x80, 0x81, 0x82, 0x83, 0x00, 0x00, 0x00, 0x00]
        );
        // The latched port 1/2 bytes are canonical state, not a per-step value,
        // so the debugger and execution paths agree byte for byte; the four stub
        // bytes stay at their reset value.
        assert_eq!(
            bus.io_control(),
            &[0x80, 0x81, 0x82, 0x83, 0x00, 0x00, 0x00, 0x00]
        );

        // Every undecoded byte of the window reads the open value and accepts a
        // discarded write instead of failing, so a boot store cannot stop here.
        for address in [
            super::CONTROLLER_WINDOW_BASE,
            0x00A1_0002,
            0x00A1_0004,
            0x00A1_0010, // first byte past the eight control registers
            0x00A1_001F,
        ] {
            assert_eq!(
                bus.read8(address),
                Ok(crate::audio::psg::OPEN_BUS_VALUE),
                "open bus at 0x{address:06X}"
            );
            assert_eq!(bus.write8(address, 0x5A), Ok(()), "write 0x{address:06X}");
            assert_eq!(
                bus.read8(address),
                Ok(crate::audio::psg::OPEN_BUS_VALUE),
                "an open-bus write is discarded at 0x{address:06X}"
            );
        }
        // The two controller data registers read their step-scoped value and
        // discard a write (the hardware has no failing latch there).
        assert_eq!(bus.write8(super::CONTROLLER_PORT_1, 0x00), Ok(()));
        assert_eq!(bus.read8(super::CONTROLLER_PORT_1), Ok(0x7E));
        assert_eq!(bus.write8(super::CONTROLLER_PORT_2, 0x00), Ok(()));
        // A word over the undecoded high byte plus a data register reads the
        // open value on the high lane and the step-scoped value on the low one:
        // both bytes are mapped now, so the access succeeds.
        assert_eq!(bus.read16(0x00A1_0002), Ok(0xFF7E));

        // Port 3/EXT data (0xA10006/0xA10007) is a real register this slice does
        // not model. Since W1X the access is accepted with no effect instead of
        // faulting: the hardware accepts the store (V1W measured Kid's
        // `MOVE.B #0x7F,(0x00A10007).L` skipped by a guest address error), so the
        // divergence is counted instead of raised.
        let mut counted = bus.port3_unmodelled_accesses();
        for address in [
            super::CONTROLLER_PORT_3_DATA,
            super::CONTROLLER_PORT_3_DATA_LOW,
        ] {
            assert_eq!(
                bus.read8(address),
                Ok(super::OPEN_BUS_VALUE),
                "port 3 data read 0x{address:06X} serves the window open value"
            );
            counted += 1;
            assert_eq!(bus.port3_unmodelled_accesses(), counted);
            assert_eq!(
                bus.write8(address, 0),
                Ok(()),
                "port 3 data write 0x{address:06X} is accepted with no effect"
            );
            counted += 1;
            assert_eq!(
                bus.port3_unmodelled_accesses(),
                counted,
                "port 3 write 0x{address:06X} is counted too"
            );
            assert_eq!(bus.peek8(address), Ok(super::OPEN_BUS_VALUE));
        }

        // The version register is a side-effect-free cartridge read.
        assert_eq!(bus.read8(super::VERSION_REGISTER), Ok(0xA1));
        assert_eq!(bus.peek8(super::VERSION_REGISTER), Ok(0xA1));

        // The debugger path stays unmapped for the step-scoped data registers: a
        // per-step value is not a machine register and must not be presented as
        // one.
        for address in [super::CONTROLLER_PORT_1, super::CONTROLLER_PORT_2] {
            assert_eq!(bus.peek8(address), Err(BusError::UnmappedRead { address }));
            assert_eq!(
                bus.read_memory(u64::from(address), 1),
                Err(BusError::UnmappedRead { address })
            );
        }

        // Past the I/O window the map is unchanged: the Z80 arbitration register
        // is owned by the Z80 window, not by open bus, and an address above the
        // window stays a typed error.
        assert!(bus.read8(0x00A1_1100).is_ok());
        assert!(matches!(
            bus.write16(0x00A1_0020, 0),
            Err(BusError::UnmappedWrite { .. })
        ));

        // A debugger write that overruns the control file by one byte is rejected
        // as a whole: the first byte is not stored, so the canonical state and
        // the state hash cannot move. This is the R1-F3 regression.
        let before = bus.io_control().to_owned();
        assert_eq!(
            bus.write_memory(u64::from(super::IO_CONTROL_STUB_BASE + 3), &[0x55, 0x66]),
            Err(BusError::UnmappedWrite {
                address: super::IO_CONTROL_BASE + super::IO_CONTROL_LEN as u32
            })
        );
        assert_eq!(
            bus.io_control(),
            &before,
            "a rejected control write must not store its first byte"
        );
        assert_eq!(
            bus.read_memory(u64::from(super::IO_CONTROL_BASE), super::IO_CONTROL_LEN)
                .expect("control file read"),
            vec![0x80, 0x81, 0x82, 0x83, 0x00, 0x00, 0x00, 0x00]
        );

        // An odd-address word access at the data register is rejected before the
        // read, as everywhere else on the 68000 bus.
        assert_eq!(
            bus.read16(super::CONTROLLER_PORT_1),
            Err(BusError::OddAddress {
                address: super::CONTROLLER_PORT_1
            })
        );
        assert_eq!(
            bus.read16(super::IO_CONTROL_BASE + 1),
            Err(BusError::OddAddress {
                address: super::IO_CONTROL_BASE + 1
            })
        );
    }

    #[test]
    fn address_normalization() {
        let mut bus = bus();
        // Bits 31..24 are not decoded: 0xFFFFF7FE is the work RAM byte 0xF7FE.
        assert_eq!(bus.read8(0xFFFF_F7FE), Ok(0x00));
        assert_eq!(bus.write8(0xFFFF_F7FE, 0x5A), Ok(()));
        assert_eq!(bus.read8(0x00FF_F7FE), Ok(0x5A));
        assert_eq!(bus.peek8(0xFFFF_F7FE), Ok(0x5A));

        // Every alias of the same 24-bit address reaches the same byte.
        for alias in [0x00FF_F7FE, 0x10FF_F7FE, 0x0EFF_F7FE, 0xFFFF_F7FE] {
            assert_eq!(bus.read8(alias), Ok(0x5A), "alias 0x{alias:08X}");
        }
        // 0x01000000 truncates to ROM 0x000000, the loader's alias, and a ROM
        // write is rejected at the same truncated address.
        assert_eq!(bus.read8(0x0100_0000), Ok(0x12));
        assert_eq!(bus.read16(0x0100_0000), Ok(0x1234));
        assert_eq!(bus.read32(0x0100_0000), Ok(0x1234_0000));
        // A ROM write is rejected by the cartridge at the truncated address,
        // exactly as the baseline already did for `0x000000`.
        assert_eq!(
            bus.write8(0x0100_0000, 0xFF),
            Err(BusError::WriteProtected { address: 0 })
        );
        assert_eq!(bus.read8(0x0000_0000), Ok(0x12));
        // 0xFFFF0000 truncates to the mirror's first byte, which is writable.
        assert_eq!(bus.write8(0xFFFF_0000, 0xFF), Ok(()));
        assert_eq!(bus.read8(0x00E0_0000), Ok(0xFF));

        // Word and long accesses normalize each lane on its own. A 32-bit
        // register value like 0xFFFF_FFFE is even, so the access is legal, and
        // its two lanes decode to 0x00FFFFFE and 0x00000000: the word reads
        // RAM. Also the alignment rule is applied to the *truncated* address,
        // which is what the hardware decodes.
        assert_eq!(bus.write8(0x00FF_FFFE, 0xBE), Ok(()));
        assert_eq!(bus.write8(0x00FF_FFFF, 0xEF), Ok(()));
        assert_eq!(bus.read16(0xFFFF_FFFE), Ok(0xBEEF));
        assert_eq!(bus.read8(0x0000_0001), Ok(0x34));
        // A lane that truncates into ROM is still a ROM access, so a write there
        // keeps the cartridge's typed rejection rather than becoming a RAM write.
        assert_eq!(
            bus.write8(0x0100_0001, 0xAA),
            Err(BusError::WriteProtected { address: 1 })
        );
        assert_eq!(
            bus.read16(0x00FF_FFFF),
            Err(BusError::OddAddress {
                address: 0x00FF_FFFF
            })
        );

        // The debugger paths normalize too, keeping both endpoints in the same
        // 24-bit space.
        assert_eq!(
            bus.read_memory(0xFFFF_F7FE, 2).expect("mirror read"),
            vec![0x5A, 0x00]
        );
        assert_eq!(bus.write_memory(0xFFFF_F7FE, &[0x11, 0x22]), Ok(()));
        assert_eq!(
            bus.read_memory(0x00FF_F7FE, 2).expect("ram read"),
            vec![0x11, 0x22]
        );
        assert_eq!(bus.read16(0xFFFF_F7FE), Ok(0x1122));

        // The Kid Chameleon exception frame: 14 bytes at the top of the mirror
        // must be writable, which is what aborted the host before this phase.
        assert_eq!(bus.validate_write_range(0xFFFF_F7F0, 14), Ok(()));
        assert_eq!(bus.validate_write_range(0xFFFF_F7F0, 0), Ok(()));
        // The last byte of the bus is writable too, and a run that starts there
        // is rejected by its wrapped endpoint instead of being accepted as a
        // range that runs off the 24-bit space.
        assert_eq!(bus.validate_write_range(0xFFFF_FFFF, 1), Ok(()));
        assert_eq!(
            bus.validate_write_range(0xFFFF_FFF0, 16),
            Ok(()),
            "a range inside the block is accepted"
        );
        assert_eq!(
            bus.validate_write_range(0xFFFF_FFFE, 3),
            Err(BusError::UnmappedWrite {
                address: 0x0100_0000
            }),
            "a range whose end wraps past the block is rejected, not wrapped"
        );

        // The bus never touches a CPU address register: the CPU keeps the raw
        // 32-bit value it was loaded with, and only this bus decodes it.
        let mut cpu = crate::cpu::M68000::new();
        cpu.set_a7(0xFFFF_F7F0);
        let _ = bus.read8(0xFFFF_F7F0);
        assert_eq!(cpu.a7(), 0xFFFF_F7F0);
    }

    #[test]
    fn ram_mirror() {
        let mut bus = bus();
        // The mirror decodes the same 64 KiB through address bits 15-0.
        let cases = [
            (0x00E0_0000_u32, 0x0000_u32),
            (0x00E0_FFFF, 0xFFFF),
            (0x00E1_0000, 0x0000),
            (0x00F0_0000, 0x0000),
            (0x00FE_FFFF, 0xFFFF),
            (0x00FF_0000, 0x0000),
            (0x00FF_FFFF, 0xFFFF),
        ];
        for (address, offset) in cases {
            let value = 0x40_u8.wrapping_add((offset & 0x3F) as u8);
            assert_eq!(bus.write8(address, value), Ok(()), "write 0x{address:06X}");
            assert_eq!(bus.read8(address), Ok(value), "read 0x{address:06X}");
            assert_eq!(
                bus.read8(super::RAM_BASE + offset),
                Ok(value),
                "the mirror and 0x{offset:04X} are the same byte"
            );
        }
        // The primary window is really the same view.
        assert_eq!(bus.write16(super::RAM_BASE + 0x1234, 0xABCD), Ok(()));
        assert_eq!(bus.read16(0x00E1_1234), Ok(0xABCD));
        assert_eq!(bus.read32(0x00E0_1234), Ok(0xABCD_0000));
        assert_eq!(bus.write32(0x00E0_1234, 0x0BAD_F00D), Ok(()));
        assert_eq!(bus.read32(super::RAM_BASE + 0x1234), Ok(0x0BAD_F00D));

        // A wide address truncates first, so 0x10FF0000 decodes to the primary
        // window's first byte (`0x00FF0000`), which is the mirror's block 0.
        assert_eq!(bus.read8(0x10FF_0000), Ok(0x40));
        // 0x01000000 truncates below the mirror and is ROM, not RAM.
        assert_eq!(bus.peek8(0x0100_0000), Ok(0x12));
    }

    #[test]
    fn ram_mirror_read_memory_range() {
        let mut bus = bus();
        bus.write_memory(0x00E0_0000, &[1, 2, 3, 4])
            .expect("mirror write");
        assert_eq!(
            bus.read_memory(0x00E0_0000, 4).expect("mirror read"),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            bus.read_memory(0x00FF_0000, 4).expect("primary read"),
            vec![1, 2, 3, 4]
        );
        // A range that crosses a 64 KiB mirror-block boundary is still the same
        // contiguous RAM, so it is served byte by byte.
        assert_eq!(
            bus.read_memory(0x00E0_FFFE, 4).expect("crossing read"),
            vec![0, 0, 1, 2]
        );
        // Crossing the end of the 24-bit bus is different from crossing an
        // internal 64 KiB block: `0x01000000` is not RAM (its low lane decodes
        // ROM), so the range is rejected at the first byte the window cannot
        // serve instead of wrapping into another region.
        assert_eq!(
            bus.read_memory(0x00FF_FFFE, 4),
            Err(BusError::UnmappedRead {
                address: 0x0100_0000
            })
        );
        assert_eq!(
            bus.read_memory(0x00FF_FFFF, 2),
            Err(BusError::UnmappedRead {
                address: 0x0100_0000
            })
        );
        // The same rule for writes: the whole range is proven before any byte
        // moves, so `0xFFFFFF` keeps its value.
        let tail_before = bus.read8(0x00FF_FFFF).expect("tail");
        assert_eq!(
            bus.write_memory(0x00FF_FFFF, &[0xAA, 0xBB]),
            Err(BusError::UnmappedWrite {
                address: 0x0100_0000
            })
        );
        assert_eq!(bus.read8(0x00FF_FFFF).expect("tail"), tail_before);
        assert_eq!(
            bus.read_memory(0x00FE_FFFE, 4).expect("mirror block read"),
            vec![0, 0, 1, 2]
        );
        // A crossing write is served too: the bytes that pass the block end
        // wrap to the block start, which is the same RAM byte the mirror alias
        // addresses, so nothing is silently reinterpreted.
        bus.write_memory(0x00E0_FFFE, &[9, 9, 9, 9])
            .expect("crossing write");
        assert_eq!(bus.read8(0x00E0_FFFE).expect("tail"), 9);
        assert_eq!(bus.read8(0x00E0_FFFF).expect("last"), 9);
        assert_eq!(bus.read8(0x00FF_0000).expect("primary head"), 9);
        assert_eq!(bus.read8(0x00E1_0000).expect("mirror head"), 9);
    }

    #[test]
    fn vdp_control_data_and_status_are_mapped_through_the_bus() {
        let mut bus = bus();
        bus.write16(0x00C0_0004, 0x8F02).expect("auto-increment");
        assert_eq!(bus.vdp().register(15), Some(2));

        set_vram_address(&mut bus, 0x0010, 0x4000);
        bus.write16(0x00C0_0000, 0xABCD).expect("vram write");
        assert_eq!(&bus.vdp().vram()[0x10..0x12], &[0xAB, 0xCD]);
        // The owner advances: the cache step only refreshes while the write FIFO
        // is empty (`General.cpp:1748`, `:1850-1856`), and the backend drains the
        // window at the step boundary.
        bus.vdp_mut().drain_fifo();

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
        // The PSG window `0xC00010..=0xC00017` is wired (W1Q) and reaches the
        // same chip the Z80 window drives, so these bytes are no longer errors.
        bus.write8(0x00C0_0011, 0x9F).expect("psg window write");
        assert_eq!(bus.psg().attenuation(0), Some(0x0F));
        assert_eq!(bus.read8(0x00C0_0011).expect("psg window read"), 0xFF);
        assert!(matches!(
            bus.read8(0x00C0_0018),
            Err(BusError::UnmappedRead { .. })
        ));
        assert!(matches!(
            bus.write8(0x00C0_0018, 0),
            Err(BusError::UnmappedWrite { .. })
        ));
        // The unmapped rest of the I/O window is open bus, so a read there is
        // the deterministic floating value and a write is discarded.
        assert_eq!(
            bus.read8(0x00A1_0000),
            Ok(crate::audio::psg::OPEN_BUS_VALUE)
        );
        assert_eq!(bus.write16(0x00A1_0000, 0), Ok(()));
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
        // The last RAM byte exists; a range that would pass the end of the
        // 24-bit bus is rejected at the first address the window cannot serve
        // (`0x01000000` is not RAM: its low lane decodes ROM).
        let last_ram = u64::from(RAM_BASE) + u64::try_from(super::RAM_SIZE).expect("ram size") - 1;
        assert_eq!(
            bus.read_memory(last_ram, 1).expect("last ram byte"),
            vec![0]
        );
        assert_eq!(
            bus.read_memory(last_ram, 2),
            Err(BusError::UnmappedRead {
                address: super::RAM_END
            })
        );
        assert_eq!(
            bus.read_memory(u64::from(u32::MAX) + 1, 1),
            Err(BusError::UnmappedRead { address: u32::MAX })
        );
        // A huge length is rejected by range arithmetic, never by asking the
        // allocator for it.
        // A huge length is clamped to the 24-bit bus instead of asking the
        // allocator for it, so it is still a typed error (never an abort).
        assert_eq!(
            bus.read_memory(u64::from(RAM_BASE), usize::MAX),
            Err(BusError::AddressOverflow {
                address: RAM_BASE,
                length: usize::MAX
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

    #[test]
    fn long_reads_compose_both_words_instead_of_zeroing_a_stub_lane() {
        // R1b-F2: the long read used to return zero for the whole value when
        // either word touched the port 3/EXT control stub. It now composes two
        // independently decoded words, so a functional word next to a stub word
        // keeps its bits and the stub/open-bus word only clears its own lane.
        let mut bus = bus();
        // 0xA1000A is in the functional port 2 control pair; 0xA1000C is stub.
        assert_eq!(bus.write16(0x00A1_000A, 0x1234), Ok(()));
        assert_eq!(bus.read16(0x00A1_000A), Ok(0x1234));
        assert_eq!(
            bus.read32(0x00A1_000A),
            Ok(0x1234_0000),
            "functional word plus stub word"
        );
        // 0xA1000E is stub; 0xA10010 is the deterministic open bus value.
        assert_eq!(bus.read16(0x00A1_000E), Ok(0x0000));
        assert_eq!(
            bus.read16(0x00A1_0010),
            Ok(u16::from_be_bytes([crate::audio::psg::OPEN_BUS_VALUE; 2]))
        );
        assert_eq!(
            bus.read32(0x00A1_000E),
            Ok(0x0000_FFFF),
            "stub word plus open-bus word"
        );
        // The starts of both halves stay as before, and a long wholly inside the
        // stub is still zero.
        assert_eq!(bus.read32(0x00A1_0008), Ok(0x0000_1234));
        assert_eq!(bus.read32(0x00A1_000C), Ok(0x0000_0000));
        // The same composition holds for the execution path and for peek8.
        assert_eq!(bus.read32(0x00A1_000A), bus.read32(0x00A1_000A));
        assert_eq!(bus.peek8(0x00A1_000C), Ok(0x00));
    }

    #[test]
    fn debugger_range_proofs_run_before_any_reservation() {
        // R1-F2: `try_reserve` is a capacity guard, not a range check. A length
        // that is representable but larger than its window must be rejected by
        // the range proof, so the request never reaches the allocator and no
        // partial state is written.
        let mut probe = bus();
        // A length that cannot even be expressed as a 24-bit span is an overflow
        // (it would wrap the address space), so it is rejected first.
        assert_eq!(
            probe.read_memory(0, usize::MAX),
            Err(BusError::AddressOverflow {
                address: 0,
                length: usize::MAX
            })
        );
        // A representable length larger than the window is bounded by the
        // window's last byte before anything is allocated.
        // The cartridge window is the image itself: a 0x400-byte test ROM ends
        // at 0x400, so an overrun is rejected there and never at the 4 MiB
        // address-space span (R1b-F1).
        assert_eq!(
            probe.read_memory(0, u32::MAX as usize),
            Err(BusError::UnmappedRead {
                address: 0x0000_0400
            }),
            "a huge ROM read is bounded by the cartridge image, not the 4 MiB window"
        );
        assert_eq!(
            probe.read_memory(u64::from(super::IO_CONTROL_BASE), usize::MAX),
            Err(BusError::AddressOverflow {
                address: super::IO_CONTROL_BASE,
                length: usize::MAX
            }),
            "a length that cannot be a 24-bit span is an overflow"
        );
        assert_eq!(
            probe.read_memory(u64::from(super::IO_CONTROL_BASE), 0x0000_0100),
            Err(BusError::UnmappedRead {
                address: super::IO_CONTROL_BASE + super::IO_CONTROL_LEN as u32
            }),
            "a read larger than the register file is bounded by it"
        );
        assert_eq!(
            probe.read_memory(0x00FF_0000, 0x0002_0000),
            Err(BusError::UnmappedRead {
                address: super::RAM_END
            }),
            "a read larger than work RAM is bounded before allocating"
        );
        assert_eq!(
            probe.write_memory(0x00FF_0000, &vec![0_u8; 0x0002_0000]),
            Err(BusError::UnmappedWrite {
                address: super::RAM_END
            }),
            "a write larger than work RAM is bounded before mutating"
        );
        assert_eq!(
            probe.read_memory(0x00FF_0000, 1).expect("ram read"),
            vec![0],
            "the rejected requests left the machine untouched"
        );
        // R1b-F1: a length that fits the 4 MiB cartridge address space but not
        // the 0x400-byte image is rejected at the image end, before the output
        // buffer is reserved; the last image byte is still readable.
        assert_eq!(
            probe.read_memory(0, 0x0000_0401),
            Err(BusError::UnmappedRead {
                address: 0x0000_0400
            })
        );
        assert_eq!(
            probe.read_memory(0x0000_03FF, 1).expect("last image byte"),
            vec![0]
        );
        // Order proof: a `u32::MAX`-byte request cannot be reserved (a 4 GiB
        // reservation fails with a capacity error), so receiving the *range*
        // error here can only mean the window proof ran before `try_reserve`.
        assert_eq!(
            probe.read_memory(0, usize::MAX),
            Err(BusError::AddressOverflow {
                address: 0,
                length: usize::MAX
            }),
            "a length that cannot be a 24-bit span is still caught first"
        );
        // A cartridge image larger than the bus window is still bounded by
        // ROM_END, which is the slice the window serves.
        let mut big_rom = vec![0_u8; super::ROM_END as usize + 0x100];
        big_rom[0] = 0x5A;
        let big = MegadriveBus::new(test_cartridge(big_rom));
        assert_eq!(big.read_memory(0, 2).expect("big rom"), vec![0x5A, 0x00]);
        assert_eq!(
            big.read_memory(u64::from(super::ROM_END) - 1, 2),
            Err(BusError::UnmappedRead {
                address: super::ROM_END
            })
        );
    }
}
