//! Minimal deterministic Sega Mega Drive / Genesis backend slice.
//!
//! **Scope:** 68000 reset vectors, the documented MC68000 instruction set
//! (including `RTE` and `MOVE.W #imm,SR`), ROM + 64 KiB work RAM, the VDP
//! status/control/VRAM port window with a derived VINT-pending status bit, a
//! per-step 3-button controller data-register subset for ports 1 and 2, a
//! deterministic master-cycle VBlank/HBlank interrupt scheduler whose
//! delivery honours the 68000 `SR` interrupt-priority mask (level 7 always,
//! otherwise strictly above the mask), a modelled cartridge (header, region,
//! Sega mapper, 64 KiB battery SRAM, TMSS lockout and version register), the
//! phase-8.3 VDP (CRAM/VSRAM, planes/window/sprites/scroll composition, DMA,
//! FIFO/CPU wait accounting, line/dot timing and VDP-owned VBlank/HBlank
//! interrupt pending/acknowledge), deterministic headless video frames,
//! versioned v5 snapshots and session integration. Debug-only observation, pure
//! CPU/VDP/interrupt/bus views and a bounded, execution-cross-checked
//! 68000-subset disassembler are available without changing canonical state.
//! The phase-8.4 Z80 core lives in [`z80`]: the documented instruction set with
//! its T-state counts, register file, interrupt acceptance (IM 0/1/2 and NMI)
//! and a versioned snapshot. [`bus`] implements the Z80 memory view (8 KiB RAM
//! mirrored through `0x3FFF`, YM2612/PSG/bank windows, a 32 KiB banked ROM
//! window) and the 68000-side 8 KiB Z80 RAM window plus the `0xA06000` bank
//! register and the `0xA11100`/`0xA11200` arbitration registers. The 68000
//! stalls by a fixed, documented number of master cycles when it touches Z80
//! RAM while the Z80 owns the bus, and [`MegadriveBackend::advance_z80`] steps
//! the core explicitly. The Z80 is not yet stepped by [`Backend::step`](chronofork_backend_api::Backend::step) in the
//! same master-cycle loop and its state is not yet in the v5 payload; both land
//! with the phase's payload-v6 task.
//! **Not implemented:** audio, concurrent Z80 execution inside a 68000 step,
//! Z80/arbitration payload serialization, TH-latched and 6-button controller
//! protocols (A/Start are not readable), SRAM banking schemes other than the Sega mapper,
//! VSRAM data-port reads, interlace, the byte-swapped physical VRAM layout,
//! dot-exact H/V counters and cycle-accurate (prefetch/bus) timing. This is
//! real-hardware groundwork, not a playable emulator.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod backend;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod cycles;
mod debug;
pub mod disasm;
pub mod inspect;
pub mod interrupts;
pub mod trace;
pub mod vdp;
pub mod z80;

pub use backend::{
    BACKEND_ID, BOOT_MARKER, BOOT_ROM_SIZE, CONFIG_MAGIC, HBLANK_VECTOR_ADDRESS, MegadriveBackend,
    MegadriveConfig, MegadriveFactory, PAYLOAD_MAGIC, PAYLOAD_VERSION, STATE_MAGIC,
    VBLANK_VECTOR_ADDRESS, boot_marker, cartridge_boot_rom, controller_rom, demo_rom,
    interrupt_rom,
};
pub use bus::{
    BusArbitration, BusError, CONTROLLER_NEUTRAL, CONTROLLER_PORT_1, CONTROLLER_PORT_2,
    CONTROLLER_WINDOW_BASE, CONTROLLER_WINDOW_END, MegadriveBus, RAM_BASE, RAM_END, RAM_SIZE,
    ROM_BASE, ROM_END, VDP_BASE, VDP_END, VDP_WINDOW_SIZE,
};
pub use cartridge::{
    Cartridge, CartridgeError, CartridgeHeader, CartridgeSystem, MAPPER_REGISTER_BASE,
    MAPPER_REGISTER_END, MAX_IMAGE_SIZE, Region, SRAM_BASE, SRAM_END, SRAM_SIZE, SramSpec,
    TMSS_REGISTER_BASE, TMSS_REGISTER_END, Tmss, TmssModel, VERSION_REGISTER,
};
pub use cpu::{
    AddressAccess, CpuError, Ea, ExceptionFrame, InstructionOutcome, M68000, Size,
    VECTOR_ADDRESS_ERROR, VECTOR_BUS_ERROR, VECTOR_CHK, VECTOR_ILLEGAL, VECTOR_IRQ_BASE,
    VECTOR_PRIVILEGE_VIOLATION, VECTOR_RESET, VECTOR_TRACE, VECTOR_TRAP_BASE, VECTOR_TRAPV,
    VECTOR_ZERO_DIVIDE, base_cycles, decode_ea, ea_address, ea_cycles, read_ea, write_ea,
};
pub use disasm::{
    DisasmError, DisasmInstruction, DisasmMemory, MAX_DISASSEMBLY_COUNT, RomMemory, WindowMemory,
    disassemble,
};
pub use inspect::{
    BusView, BusWindowView, CartridgeView, CpuView, InterruptCounterView, InterruptView,
    PendingInterruptView, VdpView,
};
pub use interrupts::{
    HBLANK_INTERRUPT_LEVEL, InterruptLevel, InterruptScheduler, PendingInterrupt, SchedulerConfig,
    SchedulerError, VBLANK_INTERRUPT_LEVEL, VideoStandard,
};
pub use trace::{InstructionTrace, TraceEntry, TraceError};
pub use vdp::{VRAM_SIZE, Vdp, VdpError};
pub use z80::{Z80, Z80Bus, Z80Error, Z80Field, Z80Outcome};

/// Test-only support for synthetic images and local commercial-ROM fixtures.
///
/// These helpers exist so the crate's synthetic ROM builders and the
/// integration tests can stamp a valid header and resolve the ignored
/// `.research/roms/` fixtures. They shape or locate ROM images before a machine
/// is constructed; they never read or mutate canonical machine state, and the
/// fixture files are never committed.
pub mod test_support {
    use std::path::PathBuf;

    use crate::cartridge::compute_checksum;

    /// Stamps a valid synthetic `SEGA MEGA DRIVE` header over `0x100..0x200`.
    ///
    /// `region` is written to the region field (`J`, `U`, `E` or a combination
    /// such as `JUE`); a caller that needs different names, a different serial or
    /// a deliberately mismatched ROM end overwrites those fields afterwards. The
    /// checksum is computed over `0x200..`, so it is unaffected by header-only
    /// edits. Returns `false` when `rom` cannot hold a full header.
    ///
    /// This is shared fixture support for the crate's synthetic ROM builders and
    /// the integration tests; it shapes the image before a machine is
    /// constructed and never touches canonical state.
    pub fn stamp_header(rom: &mut [u8], region: &str) -> bool {
        let end = match u32::try_from(rom.len()) {
            Ok(length) if length >= 0x200 => length - 1,
            _ => return false,
        };
        let checksum = compute_checksum(rom);
        let Some(header) = rom.get_mut(0x100..0x200) else {
            return false;
        };
        header.fill(0);
        header[..16].copy_from_slice(b"SEGA MEGA DRIVE ");
        header[0x20..0x30].copy_from_slice(b"CHRONOFORK TEST ");
        header[0x50..0x60].copy_from_slice(b"CHRONOFORK TEST ");
        header[0x80..0x8E].copy_from_slice(b"GM 00000000-00");
        header[0x8E..0x90].copy_from_slice(&checksum.to_be_bytes());
        header[0xA0..0xA4].copy_from_slice(&0_u32.to_be_bytes());
        header[0xA4..0xA8].copy_from_slice(&end.to_be_bytes());
        let region = region.as_bytes();
        let length = region.len().min(0x10);
        header[0xF0..0xF0 + length].copy_from_slice(&region[..length]);
        true
    }

    /// Resolves a local-only ROM fixture by name.
    ///
    /// Lookup order: `$CHRONOFORK_ROM_DIR`, then
    /// `$HOME/projects/ChronoFork/.research/roms/`, then the repository's
    /// `.research/roms/` relative to the crate manifest. Returns `None` when the
    /// file does not exist, so a fixture test can skip with an explicit reason.
    #[must_use]
    pub fn rom_path(name: &str) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(dir) = std::env::var_os("CHRONOFORK_ROM_DIR") {
            candidates.push(PathBuf::from(dir).join(name));
        }
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(
                PathBuf::from(home)
                    .join("projects/ChronoFork/.research/roms")
                    .join(name),
            );
        }
        if let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR") {
            candidates.push(
                PathBuf::from(manifest)
                    .join("../../.research/roms")
                    .join(name),
            );
        }
        candidates.into_iter().find(|path| path.is_file())
    }
}
