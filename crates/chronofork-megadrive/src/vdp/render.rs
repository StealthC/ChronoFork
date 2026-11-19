//! VDP scanline rendering.
//!
//! Phase 8.3 tasks 5–7 add per-scanline plane, window and sprite composition,
//! the priority table, shadow/highlight and the frame buffer here. Rendering
//! must stay a pure function of canonical device state and the line position:
//! it may not read host time, the environment or a window size, and two runs
//! from the same state must produce byte-identical output.
//!
//! The module is empty in task 2 because the device state and snapshot split
//! land first; no rendering behaviour exists yet and none is claimed.
