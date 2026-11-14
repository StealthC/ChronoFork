//! Minimal deterministic Sega Mega Drive / Genesis backend slice.
//!
//! **Scope:** 68000 reset vectors, a documented instruction subset, ROM + 64 KiB
//! work RAM, the VDP status/control/VRAM port window, a deterministic
//! instruction-quantum VBlank/HBlank interrupt scheduler, versioned snapshots and
//! session integration. **Not implemented:** rendering, audio, Z80, DMA,
//! controllers, mapper hardware and cycle-accurate timing. This is real-hardware
//! groundwork, not a playable emulator.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod backend;
pub mod bus;
pub mod cpu;
pub mod interrupts;
pub mod vdp;

pub use backend::{
    BACKEND_ID, HBLANK_VECTOR_ADDRESS, MegadriveBackend, MegadriveFactory, PAYLOAD_VERSION,
    VBLANK_VECTOR_ADDRESS, demo_rom,
};
pub use bus::{
    BusError, MegadriveBus, RAM_BASE, RAM_END, RAM_SIZE, ROM_BASE, ROM_END, VDP_BASE, VDP_END,
    VDP_WINDOW_SIZE,
};
pub use cpu::{CpuError, M68000};
pub use interrupts::{
    HBLANK_INTERRUPT_LEVEL, InterruptLevel, InterruptScheduler, PendingInterrupt, SchedulerConfig,
    SchedulerError, VBLANK_INTERRUPT_LEVEL, VideoStandard,
};
pub use vdp::{VRAM_SIZE, Vdp, VdpError};
