//! Side-effect-free machine inspection.

use std::collections::BTreeMap;

use chronofork_core::{StateHash, Tick};

/// A read-only view of machine state for debuggers and diagnostics.
///
/// This is intentionally a copy. Debugger clients must not hold references into
/// live backend memory, and backends must be able to produce this view without
/// changing execution state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineInspection {
    /// Current emulated tick.
    pub tick: Tick,
    /// Digest of the state at this tick.
    pub state_hash: StateHash,
    /// Whether the machine has halted.
    pub halted: bool,
    /// Named register values. A backend chooses the names it exposes.
    pub registers: BTreeMap<String, u64>,
    /// Base address of the memory window in `memory`.
    pub memory_base: u64,
    /// Contiguous memory window, starting at `memory_base`.
    pub memory: Vec<u8>,
}
