//! Pure, side-effect-free debug views over the minimal Mega Drive slice.
//!
//! This module is the single decoder for the raw `SR`, VDP, interrupt-scheduler
//! and bus state that [`crate::MegadriveBackend`] exposes. Every view is built
//! from existing accessors with `&self` borrows, copies the decoded values into
//! plain fields and never mutates a machine, device or scheduler. A CLI or a
//! future GUI prints these types instead of duplicating bit layouts, and a test
//! compares every field against the direct accessor it came from.
//!
//! The views are debug data in the same sense as the instruction trace: they are
//! not serialized, hashed or restored, and they add no canonical state.
//!
//! # Display format
//!
//! The views carry numbers and booleans, not text. Field names and formatting
//! belong to the consumer (`chronofork debug` uses its documented `key=value`
//! blocks), so this module cannot drift from a CLI format it does not own.

use chronofork_core::{StateHash, hash_bytes};

use crate::bus::{
    CONTROLLER_WINDOW_BASE, CONTROLLER_WINDOW_END, MegadriveBus, RAM_BASE, RAM_END, ROM_BASE,
    ROM_END, VDP_BASE, VDP_END,
};
use crate::cartridge::{Cartridge, CartridgeSystem, TmssModel};
use crate::cpu::{M68000, interrupt_level_deliverable};
use crate::interrupts::{InterruptLevel, InterruptScheduler, VideoStandard};
use crate::vdp::{
    AUTO_INCREMENT_REGISTER, REGISTER_COUNT, STATUS_VBLANK, STATUS_VINT_PENDING, Vdp,
};

/// 68000 trace bit (bit 15): set while instruction-by-instruction tracing is
/// enabled on the processor.
const SR_TRACE: u16 = 0x8000;
/// 68000 supervisor bit (bit 13).
const SR_SUPERVISOR: u16 = 0x2000;
/// Condition-code bit for extend (`X`, bit 4).
const SR_EXTEND: u16 = 0x0010;
/// Condition-code bit for negative (`N`, bit 3).
const SR_NEGATIVE: u16 = 0x0008;
/// Condition-code bit for zero (`Z`, bit 2).
const SR_ZERO: u16 = 0x0004;
/// Condition-code bit for overflow (`V`, bit 1).
const SR_OVERFLOW: u16 = 0x0002;
/// Condition-code bit for carry (`C`, bit 0).
const SR_CARRY: u16 = 0x0001;

/// Decoded 68000 architectural state.
///
/// Every field is derived from [`M68000`]: `supervisor`, `trace`, the
/// interrupt-priority mask and each condition-code flag decode the raw status
/// register once, so no consumer has to know a bit position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuView {
    pc: u32,
    sr: u16,
    usp: u32,
    ssp: u32,
    cycles: u64,
    halted: bool,
    stopped: bool,
    supervisor: bool,
    trace: bool,
    interrupt_mask: u8,
    x: bool,
    n: bool,
    z: bool,
    v: bool,
    c: bool,
}

impl CpuView {
    /// Decodes the current CPU state without changing it.
    #[must_use]
    pub const fn new(cpu: &M68000) -> Self {
        let sr = cpu.sr();
        Self {
            pc: cpu.pc(),
            sr,
            usp: cpu.usp(),
            ssp: cpu.ssp(),
            cycles: cpu.cycles(),
            halted: cpu.halted(),
            stopped: cpu.stopped(),
            supervisor: sr & SR_SUPERVISOR != 0,
            trace: sr & SR_TRACE != 0,
            interrupt_mask: cpu.interrupt_priority_mask(),
            x: sr & SR_EXTEND != 0,
            n: sr & SR_NEGATIVE != 0,
            z: sr & SR_ZERO != 0,
            v: sr & SR_OVERFLOW != 0,
            c: sr & SR_CARRY != 0,
        }
    }

    /// Returns the program counter.
    #[must_use]
    pub const fn pc(&self) -> u32 {
        self.pc
    }

    /// Returns the raw status register.
    #[must_use]
    pub const fn sr(&self) -> u16 {
        self.sr
    }

    /// Returns the saved user stack pointer (`USP`).
    #[must_use]
    pub const fn usp(&self) -> u32 {
        self.usp
    }

    /// Returns the saved supervisor stack pointer (`SSP`).
    #[must_use]
    pub const fn ssp(&self) -> u32 {
        self.ssp
    }

    /// Returns the accumulated 68000 cycle count.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Returns whether the CPU is halted.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }

    /// Returns whether a `STOP` instruction suspended the processor.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.stopped
    }

    /// Returns whether the processor is in supervisor mode (`S` bit).
    #[must_use]
    pub const fn supervisor(&self) -> bool {
        self.supervisor
    }

    /// Returns the status-register trace bit (`T`, bit 15).
    ///
    /// This slice does not raise trace exceptions and does not derive its
    /// separate debug instruction trace from this bit; the flag reports the
    /// architectural bit only.
    #[must_use]
    pub const fn trace(&self) -> bool {
        self.trace
    }

    /// Returns the interrupt-priority mask (`SR` bits 10-8).
    #[must_use]
    pub const fn interrupt_mask(&self) -> u8 {
        self.interrupt_mask
    }

    /// Returns the extend condition code (`X`).
    #[must_use]
    pub const fn x(&self) -> bool {
        self.x
    }

    /// Returns the negative condition code (`N`).
    #[must_use]
    pub const fn n(&self) -> bool {
        self.n
    }

    /// Returns the zero condition code (`Z`).
    #[must_use]
    pub const fn z(&self) -> bool {
        self.z
    }

    /// Returns the overflow condition code (`V`).
    #[must_use]
    pub const fn v(&self) -> bool {
        self.v
    }

    /// Returns the carry condition code (`C`).
    #[must_use]
    pub const fn c(&self) -> bool {
        self.c
    }
}

/// Decoded VDP state.
///
/// `vram_sha256` is the SHA-256 of the complete 64 KiB VRAM image, so a caller
/// can compare frames or snapshots without copying 64 KiB into a report. The
/// digest is derived on construction and is not cached by the VDP.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VdpView {
    status: u16,
    vblank: bool,
    vint_pending: bool,
    address: u16,
    auto_increment: u8,
    registers: [u8; REGISTER_COUNT],
    vram_len: usize,
    vram_sha256: StateHash,
    /// Completed VDP frame counter.
    pub frame_index: u64,
    /// Active frame width in pixels.
    pub width: u16,
    /// Active frame height in pixels.
    pub height: u16,
    /// Queued data-port words.
    pub fifo_len: usize,
    /// DMA busy state.
    pub dma_active: bool,
    /// SHA-256 of the frame the current state would produce.
    pub frame_sha256: StateHash,
}

impl VdpView {
    /// Decodes the current VDP state without changing it.
    #[must_use]
    pub fn new(vdp: &Vdp) -> Self {
        let status = vdp.status();
        let mut renderer = vdp.clone();
        let buffer = renderer.finish_frame();
        Self {
            frame_index: vdp.frame_index(),
            width: vdp.active_width(),
            height: vdp.active_height(),
            fifo_len: vdp.fifo_len(),
            dma_active: vdp.dma_active(),
            frame_sha256: buffer.checksum(),
            status,
            vblank: status & STATUS_VBLANK != 0,
            vint_pending: status & STATUS_VINT_PENDING != 0,
            address: vdp.address(),
            auto_increment: vdp.registers()[AUTO_INCREMENT_REGISTER],
            registers: *vdp.registers(),
            vram_len: vdp.vram().len(),
            vram_sha256: hash_bytes(vdp.vram()),
        }
    }

    /// Returns the ten-bit status register.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Returns the decoded VBlank status bit (bit 3).
    #[must_use]
    pub const fn vblank(&self) -> bool {
        self.vblank
    }

    /// Returns the decoded VINT-pending status bit (bit 7).
    #[must_use]
    pub const fn vint_pending(&self) -> bool {
        self.vint_pending
    }

    /// Returns the current VDP command/address register.
    #[must_use]
    pub const fn address(&self) -> u16 {
        self.address
    }

    /// Returns register `0x0F`, the data-port auto-increment.
    #[must_use]
    pub const fn auto_increment(&self) -> u8 {
        self.auto_increment
    }

    /// Returns all 24 implemented VDP registers.
    #[must_use]
    pub const fn registers(&self) -> &[u8; REGISTER_COUNT] {
        &self.registers
    }

    /// Returns the length of the VRAM image in bytes.
    #[must_use]
    pub const fn vram_len(&self) -> usize {
        self.vram_len
    }

    /// Returns the SHA-256 digest of the complete VRAM image.
    #[must_use]
    pub const fn vram_sha256(&self) -> StateHash {
        self.vram_sha256
    }
}

/// One pending interrupt request with its admission predicate.
///
/// `deliverable` applies the shared 68000 rule
/// [`interrupt_level_deliverable`] to the status register supplied when the view
/// was built: level 7 is always deliverable and every other level only when it
/// is strictly above the interrupt-priority mask. The request stays pending
/// whether or not the CPU may take it; `deliverable` is a view, not a state
/// change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingInterruptView {
    level: InterruptLevel,
    issued_tick: u64,
    deliverable: bool,
}

impl PendingInterruptView {
    /// Returns the interrupt source and 68000 level.
    #[must_use]
    pub const fn level(&self) -> InterruptLevel {
        self.level
    }

    /// Returns the virtual tick at which the request was issued.
    #[must_use]
    pub const fn issued_tick(&self) -> u64 {
        self.issued_tick
    }

    /// Returns whether the 68000 mask rule admits this request.
    #[must_use]
    pub const fn deliverable(&self) -> bool {
        self.deliverable
    }
}

/// The scheduler's request counters for one interrupt level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptCounterView {
    level: InterruptLevel,
    issued: u64,
    coalesced: u64,
}

impl InterruptCounterView {
    /// Returns the interrupt source and 68000 level.
    #[must_use]
    pub const fn level(&self) -> InterruptLevel {
        self.level
    }

    /// Returns how many requests of this level were raised since creation or
    /// restore, including requests that coalesced into a pending one.
    #[must_use]
    pub const fn issued(&self) -> u64 {
        self.issued
    }

    /// Returns how many of those requests merged into an already pending
    /// request of the same level.
    #[must_use]
    pub const fn coalesced(&self) -> u64 {
        self.coalesced
    }
}

/// Decoded interrupt-scheduler state.
///
/// `pending` lists at most one request per level in 68000 delivery-priority
/// order (VBlank before HBlank). `counters` is in
/// [`InterruptLevel::ALL`] order, HBlank then VBlank, and always holds both
/// levels even when no request is pending.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterruptView {
    standard: VideoStandard,
    lines_per_frame: u32,
    active_lines: u32,
    ticks: u64,
    frame: u64,
    line: u32,
    master_cycle_in_line: u64,
    pending: Vec<PendingInterruptView>,
    counters: [InterruptCounterView; 2],
}

impl InterruptView {
    /// Decodes the current scheduler state against `status_register`.
    ///
    /// `status_register` is the CPU `SR` word the schedule is evaluated against;
    /// it is used only for the per-pending-request `deliverable` predicate and
    /// is never stored.
    #[must_use]
    pub fn new(scheduler: &InterruptScheduler, status_register: u16) -> Self {
        let config = scheduler.config();
        let pending = scheduler
            .pending_events()
            .map(|event| PendingInterruptView {
                level: event.level(),
                issued_tick: event.issued_tick(),
                deliverable: interrupt_level_deliverable(event.level().level(), status_register),
            })
            .collect();
        let counters = InterruptLevel::ALL.map(|level| InterruptCounterView {
            level,
            issued: scheduler.requests_issued(level),
            coalesced: scheduler.requests_coalesced(level),
        });
        Self {
            standard: config.standard(),
            lines_per_frame: config.lines_per_frame(),
            active_lines: config.active_lines(),
            ticks: scheduler.ticks(),
            frame: scheduler.frame(),
            line: scheduler.line_in_frame(),
            master_cycle_in_line: scheduler.master_cycle_in_line(),
            pending,
            counters,
        }
    }

    /// Returns the configured video standard.
    #[must_use]
    pub const fn standard(&self) -> VideoStandard {
        self.standard
    }

    /// Returns the configured scanlines per frame, including blanking.
    #[must_use]
    pub const fn lines_per_frame(&self) -> u32 {
        self.lines_per_frame
    }

    /// Returns the configured active display scanlines.
    #[must_use]
    pub const fn active_lines(&self) -> u32 {
        self.active_lines
    }

    /// Returns the completed master-clock cycles.
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Returns the completed frame count.
    #[must_use]
    pub const fn frame(&self) -> u64 {
        self.frame
    }

    /// Returns the scanline currently executing inside the frame.
    #[must_use]
    pub const fn line(&self) -> u32 {
        self.line
    }

    /// Returns the master clock cycles already executed inside the current line.
    #[must_use]
    pub const fn master_cycle_in_line(&self) -> u64 {
        self.master_cycle_in_line
    }

    /// Returns the pending requests in 68000 delivery-priority order.
    #[must_use]
    pub fn pending(&self) -> &[PendingInterruptView] {
        &self.pending
    }

    /// Returns the per-level request counters in [`InterruptLevel::ALL`] order.
    #[must_use]
    pub const fn counters(&self) -> &[InterruptCounterView; 2] {
        &self.counters
    }
}

/// One mapped bus window.
///
/// `start` is inclusive and `end` is exclusive, matching the half-open
/// `base..end` constants in [`crate::bus`]. `writable` reports whether an
/// emulated write can reach the window: work RAM and the VDP port window accept
/// writes, while the cartridge image and the controller window do not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusWindowView {
    name: &'static str,
    start: u32,
    end: u32,
    writable: bool,
}

impl BusWindowView {
    /// Returns the stable window name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the inclusive first address of the window.
    #[must_use]
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// Returns the exclusive end address of the window.
    #[must_use]
    pub const fn end(&self) -> u32 {
        self.end
    }

    /// Returns whether an emulated write can reach the window.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.writable
    }
}

/// Decoded bus layout and image sizes.
///
/// The window table is the fixed address decoding of this slice, not a probe of
/// the current image: `rom_len` bounds the bytes that are actually readable
/// inside the ROM window. Work RAM always exposes [`RAM_SIZE`](crate::bus::RAM_SIZE)
/// bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusView {
    rom_len: usize,
    ram_len: usize,
    windows: [BusWindowView; 4],
}

impl BusView {
    /// Decodes the current bus layout without changing it.
    #[must_use]
    pub fn new(bus: &MegadriveBus) -> Self {
        Self {
            rom_len: bus.rom().len(),
            ram_len: bus.ram().len(),
            windows: [
                BusWindowView {
                    name: "rom",
                    start: ROM_BASE,
                    end: ROM_END,
                    writable: false,
                },
                BusWindowView {
                    name: "controller",
                    start: CONTROLLER_WINDOW_BASE,
                    end: CONTROLLER_WINDOW_END,
                    writable: false,
                },
                BusWindowView {
                    name: "vdp",
                    start: VDP_BASE,
                    end: VDP_END,
                    writable: true,
                },
                BusWindowView {
                    name: "ram",
                    start: RAM_BASE,
                    end: RAM_END,
                    writable: true,
                },
            ],
        }
    }

    /// Returns the cartridge image length in bytes.
    #[must_use]
    pub const fn rom_len(&self) -> usize {
        self.rom_len
    }

    /// Returns the work RAM length in bytes.
    #[must_use]
    pub const fn ram_len(&self) -> usize {
        self.ram_len
    }

    /// Returns the mapped windows in ascending address order.
    #[must_use]
    pub const fn windows(&self) -> &[BusWindowView; 4] {
        &self.windows
    }
}

/// Decoded cartridge header, mapper, SRAM and TMSS state.
///
/// Every field is copied from [`Cartridge`] accessors; constructing the view
/// never mutates mapper or SRAM state. The view is debug data: it is not
/// serialized, hashed or restored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CartridgeView {
    system: CartridgeSystem,
    region_bits: u8,
    region_text: String,
    domestic_name: String,
    overseas_name: String,
    serial: String,
    checksum_stored: u16,
    checksum_computed: u16,
    rom_start: u32,
    rom_end: u32,
    sram_declared: bool,
    banks: [u16; 7],
    sram_visible: bool,
    sram_write_protected: bool,
    tmss_model: TmssModel,
    tmss_locked: bool,
}

impl CartridgeView {
    /// Decodes the current cartridge state without changing it.
    #[must_use]
    pub fn new(cartridge: &Cartridge) -> Self {
        let header = cartridge.header();
        Self {
            system: header.system,
            region_bits: header.region.bits(),
            region_text: header.region.to_string(),
            domestic_name: header.domestic_name.clone(),
            overseas_name: header.overseas_name.clone(),
            serial: header.serial.clone(),
            checksum_stored: header.checksum_stored,
            checksum_computed: header.checksum_computed,
            rom_start: header.rom_start,
            rom_end: header.rom_end,
            sram_declared: header.sram.is_some(),
            banks: cartridge.banks(),
            sram_visible: cartridge.sram_visible(),
            sram_write_protected: cartridge.sram_write_protected(),
            tmss_model: cartridge.tmss().model(),
            tmss_locked: cartridge.tmss().locked(),
        }
    }

    /// Returns the console family declared by the signature.
    #[must_use]
    pub const fn system(&self) -> CartridgeSystem {
        self.system
    }

    /// Returns the raw region bitfield.
    #[must_use]
    pub const fn region_bits(&self) -> u8 {
        self.region_bits
    }

    /// Returns the region letters, for example `JUE`.
    #[must_use]
    pub fn region_text(&self) -> &str {
        &self.region_text
    }

    /// Returns the trimmed domestic name.
    #[must_use]
    pub fn domestic_name(&self) -> &str {
        &self.domestic_name
    }

    /// Returns the trimmed overseas name.
    #[must_use]
    pub fn overseas_name(&self) -> &str {
        &self.overseas_name
    }

    /// Returns the trimmed product serial.
    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Returns the checksum stored in the header.
    #[must_use]
    pub const fn checksum_stored(&self) -> u16 {
        self.checksum_stored
    }

    /// Returns the checksum computed from the image.
    #[must_use]
    pub const fn checksum_computed(&self) -> u16 {
        self.checksum_computed
    }

    /// Returns the declared ROM start offset.
    #[must_use]
    pub const fn rom_start(&self) -> u32 {
        self.rom_start
    }

    /// Returns the declared ROM end offset.
    #[must_use]
    pub const fn rom_end(&self) -> u32 {
        self.rom_end
    }

    /// Returns whether the header declares a battery-SRAM range.
    #[must_use]
    pub const fn sram_declared(&self) -> bool {
        self.sram_declared
    }

    /// Returns the seven mapped bank registers.
    #[must_use]
    pub const fn banks(&self) -> [u16; 7] {
        self.banks
    }

    /// Returns whether the SRAM window is mapped.
    #[must_use]
    pub const fn sram_visible(&self) -> bool {
        self.sram_visible
    }

    /// Returns whether the SRAM window rejects writes.
    #[must_use]
    pub const fn sram_write_protected(&self) -> bool {
        self.sram_write_protected
    }

    /// Returns the configured TMSS model.
    #[must_use]
    pub const fn tmss_model(&self) -> TmssModel {
        self.tmss_model
    }

    /// Returns whether the TMSS lockout is still active.
    #[must_use]
    pub const fn tmss_locked(&self) -> bool {
        self.tmss_locked
    }
}
