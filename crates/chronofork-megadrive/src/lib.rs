//! Minimal deterministic Sega Mega Drive / Genesis backend slice.
//!
//! **Scope:** 68000 reset vectors, a small instruction subset, ROM + 64 KiB work
//! RAM, versioned snapshots and session integration. **Not implemented:** VDP,
//! audio, Z80, controllers, mapper hardware, interrupts and cycle-accurate timing.
//! This is real-hardware groundwork, not a playable emulator.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod backend;
pub mod bus;
pub mod cpu;

pub use backend::{BACKEND_ID, MegadriveBackend, MegadriveFactory, demo_rom};
pub use bus::{BusError, MegadriveBus, RAM_BASE, RAM_END, RAM_SIZE, ROM_BASE, ROM_END};
pub use cpu::{CpuError, M68000};
