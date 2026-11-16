//! Minimal deterministic Sega Mega Drive / Genesis backend slice.
//!
//! **Scope:** 68000 reset vectors, a documented instruction subset (including
//! `RTE` and `MOVE.W #imm,SR`), ROM + 64 KiB work RAM, the VDP
//! status/control/VRAM port window with a derived VINT-pending status bit, a
//! per-step 3-button controller data-register subset for ports 1 and 2, a
//! deterministic instruction-quantum VBlank/HBlank interrupt scheduler whose
//! delivery honours the 68000 `SR` interrupt-priority mask (level 7 always,
//! otherwise strictly above the mask), versioned snapshots and session
//! integration. Debug-only observation, pure CPU/VDP/interrupt/bus views and a
//! bounded, execution-cross-checked 68000-subset disassembler are available
//! without changing canonical state. **Not implemented:** rendering, audio, Z80,
//! DMA, TH-latched and 6-button controller protocols (A/Start are not readable),
//! mapper hardware, VDP interrupt-enable/acknowledge registers and cycle-accurate
//! timing. This is real-hardware groundwork, not a playable emulator.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod backend;
pub mod bus;
pub mod cpu;
mod debug;
pub mod disasm;
pub mod inspect;
pub mod interrupts;
pub mod trace;
pub mod vdp;

pub use backend::{
    BACKEND_ID, HBLANK_VECTOR_ADDRESS, MegadriveBackend, MegadriveFactory, PAYLOAD_VERSION,
    VBLANK_VECTOR_ADDRESS, controller_rom, demo_rom, interrupt_rom,
};
pub use bus::{
    BusError, CONTROLLER_NEUTRAL, CONTROLLER_PORT_1, CONTROLLER_PORT_2, CONTROLLER_WINDOW_BASE,
    CONTROLLER_WINDOW_END, MegadriveBus, RAM_BASE, RAM_END, RAM_SIZE, ROM_BASE, ROM_END, VDP_BASE,
    VDP_END, VDP_WINDOW_SIZE,
};
pub use cpu::{CpuError, M68000};
pub use disasm::{
    DisasmError, DisasmInstruction, DisasmMemory, MAX_DISASSEMBLY_COUNT, RomMemory, WindowMemory,
    disassemble,
};
pub use inspect::{
    BusView, BusWindowView, CpuView, InterruptCounterView, InterruptView, PendingInterruptView,
    VdpView,
};
pub use interrupts::{
    DEFAULT_INSTRUCTIONS_PER_LINE, HBLANK_INTERRUPT_LEVEL, InterruptLevel, InterruptScheduler,
    PendingInterrupt, SchedulerConfig, SchedulerError, VBLANK_INTERRUPT_LEVEL, VideoStandard,
};
pub use trace::{InstructionTrace, TraceEntry, TraceError};
pub use vdp::{VRAM_SIZE, Vdp, VdpError};
