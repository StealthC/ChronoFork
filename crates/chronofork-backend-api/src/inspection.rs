//! Cheap status and on-demand machine inspection.

use std::collections::BTreeMap;

use chronofork_core::Tick;

/// Upper bound the register maps must respect in every backend.
pub const MAX_STATUS_REGISTERS: usize = 64;

/// Cheap status: no digest and no memory copy.
///
/// This is the view used for listing sessions and for periodic observation: it must be
/// produced from the backend's own fields without serializing state or copying RAM. The
/// 68000/core register file and the device pseudo-registers are kept in **separate**
/// maps, so a consumer cannot treat a naming convention as the separation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineStatus {
    /// Current emulated tick.
    pub tick: Tick,
    /// Whether the machine has halted.
    pub halted: bool,
    /// The 68000/core register file (`d0..d7`, `a0..a7`, `pc`, `sr`, ...).
    pub cpu_registers: BTreeMap<String, u64>,
    /// Device pseudo-registers (`vdp_status`, `mapper.bank*`, `sram.*`, `tmss.*`, ...).
    pub device_registers: BTreeMap<String, u64>,
    /// Frame index at this tick (the render boundary identity).
    pub frame_index: u64,
}

impl MachineStatus {
    /// Returns the merged view for consumers that want one map.
    #[must_use]
    pub fn registers(&self) -> BTreeMap<String, u64> {
        let mut merged = self.cpu_registers.clone();
        merged.extend(self.device_registers.clone());
        merged
    }

    /// Returns the total number of exposed registers.
    #[must_use]
    pub fn register_count(&self) -> usize {
        self.cpu_registers.len() + self.device_registers.len()
    }
}

/// A contiguous memory window requested by a caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryWindow {
    /// Base address of the window.
    pub base: u64,
    /// Window bytes.
    pub bytes: Vec<u8>,
}

/// On-demand inspection view: registers plus, only when asked for, a memory window.
///
/// It never computes a digest. A caller that needs one calls
/// [`Backend::state_hash`](crate::Backend::state_hash) explicitly, once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineInspection {
    /// Current emulated tick.
    pub tick: Tick,
    /// Whether the machine has halted.
    pub halted: bool,
    /// Frame index at this tick.
    pub frame_index: u64,
    /// The 68000/core register file.
    pub cpu_registers: BTreeMap<String, u64>,
    /// Device pseudo-registers.
    pub device_registers: BTreeMap<String, u64>,
    /// The memory window, only when the caller asked for it.
    pub memory: Option<MemoryWindow>,
}

impl MachineInspection {
    /// Returns the merged register view.
    #[must_use]
    pub fn registers(&self) -> BTreeMap<String, u64> {
        let mut merged = self.cpu_registers.clone();
        merged.extend(self.device_registers.clone());
        merged
    }

    /// Returns the total number of exposed registers.
    #[must_use]
    pub fn register_count(&self) -> usize {
        self.cpu_registers.len() + self.device_registers.len()
    }

    /// Attaches an opt-in memory window.
    #[must_use]
    pub fn with_memory(mut self, window: MemoryWindow) -> Self {
        self.memory = Some(window);
        self
    }
}
