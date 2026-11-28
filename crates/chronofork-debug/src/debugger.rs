//! The host debugger and its deterministic run loop.

use std::collections::BTreeMap;
use std::fmt;

use chronofork_backend_api::{
    Backend, DebugConfig, DebugEvent, DebugEventKind, DebugEventLog, MachineInspection,
    WatchpointSpec,
};
use chronofork_core::{InputFrame, StateHash, Tick};

use crate::error::{DebugError, InvalidLogMessage, TriggerKind};
use crate::run::{DebugRun, StopReason};
use crate::trigger::{
    BreakpointId, BreakpointSpec, Condition, ConditionOperand, EventBreakpointId, EventFilter,
    Location, LogEntry, Logpoint, LogpointId, TriggerId, WatchpointId,
};
use crate::{
    DEFAULT_EVENT_CAPACITY, DEFAULT_LOG_CAPACITY, MAX_BREAKPOINTS, MAX_EVENT_BREAKPOINTS,
    MAX_EVENT_CAPACITY, MAX_LOG_MESSAGE_BYTES, MAX_LOGPOINTS, MAX_RUN_STEP_LIMIT, MAX_WATCHPOINTS,
};

/// Register name every location match reads from the inspection.
const PROGRAM_COUNTER_REGISTER: &str = "pc";
/// Largest accepted memory condition, in bytes.
const MAX_MEMORY_CONDITION_BYTES: u8 = 8;

#[derive(Debug)]
struct BreakpointEntry {
    spec: BreakpointSpec,
    enabled: bool,
}

#[derive(Debug)]
struct WatchpointEntry {
    spec: WatchpointSpec,
    enabled: bool,
}

#[derive(Debug)]
struct EventBreakpointEntry {
    filter: EventFilter,
    enabled: bool,
}

#[derive(Debug)]
struct LogpointEntry {
    point: Logpoint,
    enabled: bool,
    hits: u64,
}

/// A host debugger over one borrowed backend.
///
/// The debugger owns the trigger tables, the logpoint ring and the event cursor;
/// it borrows the backend only for execution and observation. See the
/// [crate documentation](crate) for the run order and the bounds.
pub struct Debugger<'a> {
    backend: &'a mut dyn Backend,
    event_capacity: usize,
    next_breakpoint_id: u32,
    next_watchpoint_id: u32,
    next_event_breakpoint_id: u32,
    next_logpoint_id: u32,
    breakpoints: BTreeMap<u32, BreakpointEntry>,
    watchpoints: BTreeMap<u32, WatchpointEntry>,
    event_breakpoints: BTreeMap<u32, EventBreakpointEntry>,
    logpoints: BTreeMap<u32, LogpointEntry>,
    log_entries: Vec<LogEntry>,
    log_dropped: u64,
    event_cursor: u64,
}

impl fmt::Debug for Debugger<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Debugger")
            .field("event_capacity", &self.event_capacity)
            .field("breakpoints", &self.breakpoints.len())
            .field("watchpoints", &self.watchpoints.len())
            .field("event_breakpoints", &self.event_breakpoints.len())
            .field("logpoints", &self.logpoints.len())
            .field("log_entries", &self.log_entries.len())
            .field("log_dropped", &self.log_dropped)
            .field("event_cursor", &self.event_cursor)
            .finish_non_exhaustive()
    }
}

impl<'a> Debugger<'a> {
    /// Creates a debugger with the default event capacity
    /// ([`DEFAULT_EVENT_CAPACITY`]).
    ///
    /// When the backend exposes debug observation, an empty watchpoint table is
    /// installed immediately, which starts the bounded event ring and discards
    /// any previously retained events.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DebugError`] when the backend rejects the initial
    /// observation configuration.
    pub fn new(backend: &'a mut dyn Backend) -> Result<Self, DebugError> {
        Self::with_event_capacity(backend, DEFAULT_EVENT_CAPACITY)
    }

    /// Creates a debugger with an explicit event-ring capacity in
    /// `1..=MAX_EVENT_CAPACITY`.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidEventCapacity`] outside that range and a
    /// typed [`DebugError::Backend`] when the backend rejects the configuration.
    pub fn with_event_capacity(
        backend: &'a mut dyn Backend,
        capacity: usize,
    ) -> Result<Self, DebugError> {
        if capacity == 0 || capacity > MAX_EVENT_CAPACITY {
            return Err(DebugError::InvalidEventCapacity(capacity));
        }
        let mut debugger = Self {
            backend,
            event_capacity: capacity,
            next_breakpoint_id: 1,
            next_watchpoint_id: 1,
            next_event_breakpoint_id: 1,
            next_logpoint_id: 1,
            breakpoints: BTreeMap::new(),
            watchpoints: BTreeMap::new(),
            event_breakpoints: BTreeMap::new(),
            logpoints: BTreeMap::new(),
            log_entries: Vec::new(),
            log_dropped: 0,
            event_cursor: 0,
        };
        debugger.install_observation()?;
        Ok(debugger)
    }

    /// Adds a breakpoint that stops **before** a matching instruction executes.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidRange`] for an inverted location range,
    /// [`DebugError::InvalidConditionLength`] for a memory condition outside
    /// `1..=8` bytes, [`DebugError::UnknownRegister`] for a condition naming an
    /// absent register, [`DebugError::MissingProgramCounter`] when the
    /// inspection has no `pc` register, [`DebugError::TableFull`] when
    /// [`MAX_BREAKPOINTS`] is reached, and [`DebugError::Backend`] when the
    /// backend inspection fails.
    pub fn add_breakpoint(&mut self, spec: BreakpointSpec) -> Result<BreakpointId, DebugError> {
        validate_location(spec.location)?;
        if self.breakpoints.len() >= MAX_BREAKPOINTS {
            return Err(DebugError::TableFull(TriggerKind::Breakpoint));
        }
        let inspection = self.inspect()?;
        require_program_counter(&inspection)?;
        if let Some(condition) = &spec.condition {
            validate_condition(condition, &inspection)?;
        }
        let id = next_id(&self.breakpoints, &mut self.next_breakpoint_id);
        self.breakpoints.insert(
            id,
            BreakpointEntry {
                spec,
                enabled: true,
            },
        );
        Ok(BreakpointId::new(id))
    }

    /// Adds a watchpoint installed in the backend observation configuration.
    ///
    /// The backend must expose
    /// [`debug_access`](chronofork_backend_api::Backend::debug_access); host
    /// watchpoints are matched by the backend bus, never approximated by a memory
    /// diff. The host id is distinct from the spec's backend id.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::WatchpointsUnsupported`] when the backend cannot
    /// observe its bus, [`DebugError::DuplicateWatchpointId`] when the spec's
    /// backend id is already installed, [`DebugError::TableFull`] at
    /// [`MAX_WATCHPOINTS`], and a typed [`DebugError::Backend`] when the backend
    /// rejects the configuration.
    pub fn add_watchpoint(&mut self, spec: WatchpointSpec) -> Result<WatchpointId, DebugError> {
        if self.backend.debug_access().is_none() {
            return Err(DebugError::WatchpointsUnsupported);
        }
        if self.watchpoints.len() >= MAX_WATCHPOINTS {
            return Err(DebugError::TableFull(TriggerKind::Watchpoint));
        }
        if self
            .watchpoints
            .values()
            .any(|entry| entry.spec.id() == spec.id())
        {
            return Err(DebugError::DuplicateWatchpointId(spec.id()));
        }
        let id = next_id(&self.watchpoints, &mut self.next_watchpoint_id);
        self.watchpoints.insert(
            id,
            WatchpointEntry {
                spec,
                enabled: true,
            },
        );
        if let Err(error) = self.install_observation() {
            self.watchpoints.remove(&id);
            return Err(error);
        }
        Ok(WatchpointId::new(id))
    }

    /// Adds an event breakpoint that stops **after** a step whose retained
    /// events match `filter`.
    ///
    /// The trigger is host-owned. Without backend observation no event can be
    /// produced, so [`Debugger::retained_events`] and
    /// [`Debugger::add_watchpoint`] report
    /// [`DebugError::WatchpointsUnsupported`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::TableFull`] when [`MAX_EVENT_BREAKPOINTS`] is
    /// reached.
    pub fn add_event_breakpoint(
        &mut self,
        filter: EventFilter,
    ) -> Result<EventBreakpointId, DebugError> {
        if self.event_breakpoints.len() >= MAX_EVENT_BREAKPOINTS {
            return Err(DebugError::TableFull(TriggerKind::EventBreakpoint));
        }
        let id = next_id(&self.event_breakpoints, &mut self.next_event_breakpoint_id);
        self.event_breakpoints.insert(
            id,
            EventBreakpointEntry {
                filter,
                enabled: true,
            },
        );
        Ok(EventBreakpointId::new(id))
    }

    /// Adds a logpoint that appends a bounded log entry and never stops.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidRange`] for an inverted location range,
    /// [`DebugError::InvalidLogMessage`] when the message is empty, over
    /// [`MAX_LOG_MESSAGE_BYTES`] or contains a line break,
    /// [`DebugError::MissingProgramCounter`] when the inspection has no `pc`
    /// register, [`DebugError::TableFull`] at [`MAX_LOGPOINTS`], and a typed
    /// [`DebugError::Backend`] when the backend inspection fails.
    pub fn add_logpoint(
        &mut self,
        location: Location,
        message: &str,
    ) -> Result<LogpointId, DebugError> {
        validate_location(location)?;
        validate_log_message(message)?;
        if self.logpoints.len() >= MAX_LOGPOINTS {
            return Err(DebugError::TableFull(TriggerKind::Logpoint));
        }
        let inspection = self.inspect()?;
        require_program_counter(&inspection)?;
        let id = next_id(&self.logpoints, &mut self.next_logpoint_id);
        self.logpoints.insert(
            id,
            LogpointEntry {
                point: Logpoint::new(location, message.to_owned()),
                enabled: true,
                hits: 0,
            },
        );
        Ok(LogpointId::new(id))
    }

    /// Removes a breakpoint.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::UnknownTrigger`] when no breakpoint has the id.
    pub fn remove_breakpoint(&mut self, id: BreakpointId) -> Result<(), DebugError> {
        self.breakpoints
            .remove(&id.value())
            .map(|_| ())
            .ok_or(DebugError::UnknownTrigger(TriggerId::Breakpoint(id)))
    }

    /// Removes a watchpoint and reinstalls the remaining backend observation
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::UnknownTrigger`] when no watchpoint has the id and
    /// a typed [`DebugError::Backend`] when the backend rejects the new
    /// configuration; the watchpoint table is left unchanged on either error.
    pub fn remove_watchpoint(&mut self, id: WatchpointId) -> Result<(), DebugError> {
        let key = id.value();
        let Some(entry) = self.watchpoints.remove(&key) else {
            return Err(DebugError::UnknownTrigger(TriggerId::Watchpoint(id)));
        };
        if let Err(error) = self.install_observation() {
            self.watchpoints.insert(key, entry);
            return Err(error);
        }
        Ok(())
    }

    /// Removes an event breakpoint.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::UnknownTrigger`] when no event breakpoint has the
    /// id.
    pub fn remove_event_breakpoint(&mut self, id: EventBreakpointId) -> Result<(), DebugError> {
        self.event_breakpoints
            .remove(&id.value())
            .map(|_| ())
            .ok_or(DebugError::UnknownTrigger(TriggerId::EventBreakpoint(id)))
    }

    /// Removes a logpoint.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::UnknownTrigger`] when no logpoint has the id.
    pub fn remove_logpoint(&mut self, id: LogpointId) -> Result<(), DebugError> {
        self.logpoints
            .remove(&id.value())
            .map(|_| ())
            .ok_or(DebugError::UnknownTrigger(TriggerId::Logpoint(id)))
    }

    /// Enables or disables any installed trigger.
    ///
    /// Disabling a watchpoint removes it from the installed backend
    /// observation configuration; enabling it installs it again.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::UnknownTrigger`] when no trigger has the id and a
    /// typed [`DebugError::Backend`] when the backend rejects a watchpoint
    /// reconfiguration; the watchpoint table is left unchanged on either error.
    pub fn set_enabled(&mut self, id: TriggerId, enabled: bool) -> Result<(), DebugError> {
        match id {
            TriggerId::Breakpoint(id) => {
                let Some(entry) = self.breakpoints.get_mut(&id.value()) else {
                    return Err(DebugError::UnknownTrigger(TriggerId::Breakpoint(id)));
                };
                entry.enabled = enabled;
                Ok(())
            }
            TriggerId::EventBreakpoint(id) => {
                let Some(entry) = self.event_breakpoints.get_mut(&id.value()) else {
                    return Err(DebugError::UnknownTrigger(TriggerId::EventBreakpoint(id)));
                };
                entry.enabled = enabled;
                Ok(())
            }
            TriggerId::Logpoint(id) => {
                let Some(entry) = self.logpoints.get_mut(&id.value()) else {
                    return Err(DebugError::UnknownTrigger(TriggerId::Logpoint(id)));
                };
                entry.enabled = enabled;
                Ok(())
            }
            TriggerId::Watchpoint(id) => self.set_watchpoint_enabled(id, enabled),
        }
    }

    /// Returns a read-only snapshot of the installed trigger tables.
    #[must_use]
    pub fn triggers(&self) -> TriggerSnapshot<'_> {
        TriggerSnapshot {
            breakpoints: &self.breakpoints,
            watchpoints: &self.watchpoints,
            event_breakpoints: &self.event_breakpoints,
            logpoints: &self.logpoints,
        }
    }

    /// Returns a side-effect-free inspection of the borrowed backend.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DebugError::Backend`] when the backend cannot inspect
    /// its state.
    /// Returns the canonical state digest explicitly (inspection no longer carries it).
    ///
    /// # Errors
    ///
    /// Propagates the backend digest error.
    pub fn state_hash(&self) -> Result<chronofork_core::StateHash, DebugError> {
        self.backend.state_hash().map_err(DebugError::Backend)
    }

    pub fn inspect(&self) -> Result<MachineInspection, DebugError> {
        self.backend.inspect().map_err(DebugError::Backend)
    }

    /// Reads emulated memory through the side-effect-free debugger path.
    ///
    /// A debugger read is not an emulated bus access: it never produces a debug
    /// event and never triggers a watchpoint.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DebugError::Backend`] when the backend rejects the read.
    pub fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, DebugError> {
        self.backend
            .read_memory(address, length)
            .map_err(DebugError::Backend)
    }

    /// Writes emulated memory through the debugger path.
    ///
    /// A debugger write is not an emulated bus access: it never produces a debug
    /// event and never triggers a watchpoint.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DebugError::Backend`] when the backend rejects the
    /// write.
    pub fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), DebugError> {
        self.backend
            .write_memory(address, data)
            .map_err(DebugError::Backend)
    }

    /// Runs up to `step_limit` neutral-input instructions and returns the stop.
    ///
    /// The exact ordering is documented on [`Debugger`] and in ADR-0010 section
    /// 3.2. `step_limit` is in `1..=MAX_RUN_STEP_LIMIT`.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::InvalidStepLimit`] for an out-of-range limit,
    /// [`DebugError::MissingProgramCounter`] when a breakpoint or logpoint needs
    /// `pc` and the inspection lacks it, [`DebugError::UnknownRegister`] when a
    /// condition register disappeared, [`DebugError::InvalidConditionLength`]
    /// for a stored memory condition outside `1..=8` bytes, and a typed
    /// [`DebugError::Backend`] for inspection, memory or step failures.
    pub fn run(&mut self, step_limit: u64) -> Result<DebugRun, DebugError> {
        if step_limit == 0 || step_limit > MAX_RUN_STEP_LIMIT {
            return Err(DebugError::InvalidStepLimit(step_limit));
        }
        let mut steps = 0_u64;
        loop {
            let inspection = self.inspect()?;
            if let Some(id) = self.matching_breakpoint(&inspection)? {
                return Ok(self.stop_report(
                    StopReason::Breakpoint { id },
                    steps,
                    inspection.tick,
                    self.backend.state_hash().map_err(DebugError::Backend)?,
                    inspection.halted,
                    Vec::new(),
                ));
            }
            self.append_logpoints(&inspection)?;
            if inspection.halted {
                return Ok(self.stop_report(
                    StopReason::Halted,
                    steps,
                    inspection.tick,
                    self.backend.state_hash().map_err(DebugError::Backend)?,
                    true,
                    Vec::new(),
                ));
            }
            if steps == step_limit {
                return Ok(self.stop_report(
                    StopReason::StepLimit,
                    steps,
                    inspection.tick,
                    self.backend.state_hash().map_err(DebugError::Backend)?,
                    inspection.halted,
                    Vec::new(),
                ));
            }

            let outcome = self
                .backend
                .step(InputFrame::NEUTRAL)
                .map_err(DebugError::Backend)?;
            steps += 1;
            let events = self.collect_events();
            let matched_events = self.matching_events(&events);
            let watchpoint_ids = self.watchpoint_hits(&events);
            if watchpoint_ids.is_empty() && self.first_matching_event_breakpoint(&events).is_none()
            {
                continue;
            }
            // A stop report needs a digest, and the ordinary outcome no longer carries
            // one, so the debugger asks the canonical source explicitly.
            let state_hash = self.backend.state_hash().map_err(DebugError::Backend)?;
            if !watchpoint_ids.is_empty() {
                return Ok(self.stop_report(
                    StopReason::Watchpoint {
                        ids: watchpoint_ids,
                    },
                    steps,
                    outcome.tick,
                    state_hash,
                    outcome.halted,
                    matched_events,
                ));
            }
            if let Some(id) = self.first_matching_event_breakpoint(&events) {
                return Ok(self.stop_report(
                    StopReason::Event { id },
                    steps,
                    outcome.tick,
                    state_hash,
                    outcome.halted,
                    matched_events,
                ));
            }
        }
    }

    /// Returns every event currently retained by the backend event ring,
    /// oldest first.
    ///
    /// Consumed events stay readable until the bounded ring evicts them; the run
    /// cursor is not reset.
    ///
    /// # Errors
    ///
    /// Returns [`DebugError::WatchpointsUnsupported`] when the backend exposes no
    /// debug observation or no event ring.
    pub fn retained_events(&mut self) -> Result<Vec<DebugEvent>, DebugError> {
        let Some(access) = self.backend.debug_access() else {
            return Err(DebugError::WatchpointsUnsupported);
        };
        let Some(log) = access.debug_event_log() else {
            return Err(DebugError::WatchpointsUnsupported);
        };
        Ok(log.entries().collect())
    }

    /// Returns the logpoint entries currently retained by the bounded log ring.
    #[must_use]
    pub fn log_entries(&self) -> &[LogEntry] {
        &self.log_entries
    }

    fn set_watchpoint_enabled(
        &mut self,
        id: WatchpointId,
        enabled: bool,
    ) -> Result<(), DebugError> {
        let key = id.value();
        let Some(entry) = self.watchpoints.get(&key) else {
            return Err(DebugError::UnknownTrigger(TriggerId::Watchpoint(id)));
        };
        let previous = entry.enabled;
        if let Some(entry) = self.watchpoints.get_mut(&key) {
            entry.enabled = enabled;
        }
        if let Err(error) = self.install_observation() {
            if let Some(entry) = self.watchpoints.get_mut(&key) {
                entry.enabled = previous;
            }
            return Err(error);
        }
        Ok(())
    }

    /// Installs the enabled watchpoints and the event capacity in the backend.
    ///
    /// A backend without observation is accepted only while no watchpoint is
    /// installed; a run over such a backend simply records no events.
    fn install_observation(&mut self) -> Result<(), DebugError> {
        let watchpoints: Vec<WatchpointSpec> = self
            .watchpoints
            .values()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.spec)
            .collect();
        let config =
            DebugConfig::new(self.event_capacity, watchpoints).map_err(DebugError::Backend)?;
        let Some(access) = self.backend.debug_access() else {
            return if self.watchpoints.is_empty() {
                Ok(())
            } else {
                Err(DebugError::WatchpointsUnsupported)
            };
        };
        access
            .configure_debug(config)
            .map_err(DebugError::Backend)?;
        self.event_cursor = 0;
        Ok(())
    }

    /// Returns the first enabled breakpoint in id order whose location and
    /// condition hold for the pre-step inspection.
    fn matching_breakpoint(
        &self,
        inspection: &MachineInspection,
    ) -> Result<Option<BreakpointId>, DebugError> {
        if self.breakpoints.is_empty() {
            return Ok(None);
        }
        let pc = program_counter(inspection)?;
        for (id, entry) in &self.breakpoints {
            if !entry.enabled || !entry.spec.location.matches(pc) {
                continue;
            }
            let holds = match &entry.spec.condition {
                Some(condition) => self.condition_holds(condition, inspection)?,
                None => true,
            };
            if holds {
                return Ok(Some(BreakpointId::new(*id)));
            }
        }
        Ok(None)
    }

    /// Appends one bounded log entry for every enabled matching logpoint, in id
    /// order. A logpoint never changes the stop reason.
    fn append_logpoints(&mut self, inspection: &MachineInspection) -> Result<(), DebugError> {
        if self.logpoints.is_empty() {
            return Ok(());
        }
        let pc = program_counter(inspection)?;
        let mut pending = Vec::new();
        for (id, entry) in &mut self.logpoints {
            if !entry.enabled || !entry.point.location().matches(pc) {
                continue;
            }
            entry.hits = entry.hits.saturating_add(1);
            pending.push(LogEntry {
                logpoint_id: LogpointId::new(*id),
                tick: inspection.tick,
                pc,
                hits: entry.hits,
                message: entry.point.message().to_owned(),
            });
        }
        for entry in pending {
            self.record_log(entry);
        }
        Ok(())
    }

    fn record_log(&mut self, entry: LogEntry) {
        if self.log_entries.len() >= DEFAULT_LOG_CAPACITY {
            self.log_entries.remove(0);
            self.log_dropped = self.log_dropped.saturating_add(1);
        }
        self.log_entries.push(entry);
    }

    /// Returns whether the condition holds for the current inspection.
    ///
    /// Memory conditions read through the side-effect-free debugger path and
    /// compare `1..=8` bytes as one big-endian value.
    fn condition_holds(
        &self,
        condition: &Condition,
        inspection: &MachineInspection,
    ) -> Result<bool, DebugError> {
        let actual = match &condition.operand {
            ConditionOperand::Register(name) => *inspection
                .registers()
                .get(name)
                .ok_or_else(|| DebugError::UnknownRegister(name.clone()))?,
            ConditionOperand::Memory { address, length } => {
                if *length == 0 || *length > MAX_MEMORY_CONDITION_BYTES {
                    return Err(DebugError::InvalidConditionLength(*length));
                }
                let bytes = self
                    .backend
                    .read_memory(u64::from(*address), usize::from(*length))
                    .map_err(DebugError::Backend)?;
                bytes
                    .iter()
                    .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte))
            }
        };
        Ok(condition.relation.holds(actual, condition.value))
    }

    /// Removes the events recorded since the previous cursor from the run
    /// cursor and returns them.
    fn collect_events(&mut self) -> Vec<DebugEvent> {
        let Some(access) = self.backend.debug_access() else {
            return Vec::new();
        };
        let Some(log) = access.debug_event_log() else {
            return Vec::new();
        };
        let first = log.first_index().unwrap_or_else(|| log.next_index());
        let cursor = self.event_cursor.max(first);
        let dropped = log.dropped();
        let offset = usize::try_from(cursor.saturating_sub(dropped)).unwrap_or(usize::MAX);
        let start = offset.min(log.len());
        let collected: Vec<DebugEvent> = log.entries().skip(start).collect();
        let next = log.next_index();
        self.event_cursor = next;
        collected
    }

    /// Returns every event of the step that matches at least one enabled event
    /// breakpoint, in recording order.
    fn matching_events(&self, events: &[DebugEvent]) -> Vec<DebugEvent> {
        events
            .iter()
            .copied()
            .filter(|event| {
                self.event_breakpoints
                    .values()
                    .any(|entry| entry.enabled && entry.filter.matches(event.kind))
            })
            .collect()
    }

    /// Returns the first enabled event breakpoint in id order that matches any
    /// event of the step.
    fn first_matching_event_breakpoint(&self, events: &[DebugEvent]) -> Option<EventBreakpointId> {
        self.event_breakpoints
            .iter()
            .find(|(_, entry)| {
                entry.enabled && events.iter().any(|event| entry.filter.matches(event.kind))
            })
            .map(|(id, _)| EventBreakpointId::new(*id))
    }

    /// Returns every enabled host watchpoint that a `Watchpoint` event of the
    /// step names, deduplicated in first-hit order.
    fn watchpoint_hits(&self, events: &[DebugEvent]) -> Vec<WatchpointId> {
        let mut hits = Vec::new();
        for event in events {
            let DebugEventKind::Watchpoint { watchpoint_id, .. } = event.kind else {
                continue;
            };
            for (id, entry) in &self.watchpoints {
                if entry.enabled && entry.spec.id() == watchpoint_id {
                    let hit = WatchpointId::new(*id);
                    if !hits.contains(&hit) {
                        hits.push(hit);
                    }
                }
            }
        }
        hits
    }

    fn event_dropped(&mut self) -> u64 {
        match self.backend.debug_access() {
            Some(access) => access.debug_event_log().map_or(0, DebugEventLog::dropped),
            None => 0,
        }
    }

    fn stop_report(
        &mut self,
        reason: StopReason,
        steps: u64,
        tick: Tick,
        state_hash: StateHash,
        halted: bool,
        matched_events: Vec<DebugEvent>,
    ) -> DebugRun {
        let dropped_events = self.event_dropped();
        let log_entries = self.log_entries.clone();
        let dropped_log_entries = self.log_dropped;
        DebugRun {
            reason,
            steps,
            tick,
            state_hash,
            halted,
            matched_events,
            dropped_events,
            log_entries,
            dropped_log_entries,
        }
    }
}

/// A read-only view of the installed trigger tables.
///
/// Iterators are in trigger id order and include disabled triggers.
#[derive(Clone, Copy, Debug)]
pub struct TriggerSnapshot<'a> {
    breakpoints: &'a BTreeMap<u32, BreakpointEntry>,
    watchpoints: &'a BTreeMap<u32, WatchpointEntry>,
    event_breakpoints: &'a BTreeMap<u32, EventBreakpointEntry>,
    logpoints: &'a BTreeMap<u32, LogpointEntry>,
}

impl<'a> TriggerSnapshot<'a> {
    /// Iterates `(id, spec, enabled)` for every breakpoint.
    pub fn breakpoints(
        &self,
    ) -> impl ExactSizeIterator<Item = (BreakpointId, &'a BreakpointSpec, bool)> + 'a {
        let entries: &'a BTreeMap<u32, BreakpointEntry> = self.breakpoints;
        entries
            .iter()
            .map(|(id, entry)| (BreakpointId::new(*id), &entry.spec, entry.enabled))
    }

    /// Iterates `(id, spec, enabled)` for every host watchpoint.
    pub fn watchpoints(
        &self,
    ) -> impl ExactSizeIterator<Item = (WatchpointId, &'a WatchpointSpec, bool)> + 'a {
        let entries: &'a BTreeMap<u32, WatchpointEntry> = self.watchpoints;
        entries
            .iter()
            .map(|(id, entry)| (WatchpointId::new(*id), &entry.spec, entry.enabled))
    }

    /// Iterates `(id, filter, enabled)` for every event breakpoint.
    pub fn event_breakpoints(
        &self,
    ) -> impl ExactSizeIterator<Item = (EventBreakpointId, &'a EventFilter, bool)> + 'a {
        let entries: &'a BTreeMap<u32, EventBreakpointEntry> = self.event_breakpoints;
        entries
            .iter()
            .map(|(id, entry)| (EventBreakpointId::new(*id), &entry.filter, entry.enabled))
    }

    /// Iterates `(id, logpoint, enabled, hits)` for every logpoint.
    pub fn logpoints(
        &self,
    ) -> impl ExactSizeIterator<Item = (LogpointId, &'a Logpoint, bool, u64)> + 'a {
        let entries: &'a BTreeMap<u32, LogpointEntry> = self.logpoints;
        entries.iter().map(|(id, entry)| {
            (
                LogpointId::new(*id),
                &entry.point,
                entry.enabled,
                entry.hits,
            )
        })
    }
}

fn validate_location(location: Location) -> Result<(), DebugError> {
    match location {
        Location::Address(_) => Ok(()),
        Location::Range { start, end } if start > end => {
            Err(DebugError::InvalidRange { start, end })
        }
        Location::Range { .. } => Ok(()),
    }
}

fn validate_condition(
    condition: &Condition,
    inspection: &MachineInspection,
) -> Result<(), DebugError> {
    match &condition.operand {
        ConditionOperand::Register(name) => {
            if inspection.registers().contains_key(name) {
                Ok(())
            } else {
                Err(DebugError::UnknownRegister(name.clone()))
            }
        }
        ConditionOperand::Memory { length, .. } => {
            if *length == 0 || *length > MAX_MEMORY_CONDITION_BYTES {
                Err(DebugError::InvalidConditionLength(*length))
            } else {
                Ok(())
            }
        }
    }
}

fn validate_log_message(message: &str) -> Result<(), DebugError> {
    if message.is_empty() {
        return Err(DebugError::InvalidLogMessage(InvalidLogMessage::Empty));
    }
    if message.len() > MAX_LOG_MESSAGE_BYTES {
        return Err(DebugError::InvalidLogMessage(InvalidLogMessage::TooLong {
            length: message.len(),
        }));
    }
    if message.contains(['\n', '\r']) {
        return Err(DebugError::InvalidLogMessage(
            InvalidLogMessage::ContainsLineBreak,
        ));
    }
    Ok(())
}

fn program_counter(inspection: &MachineInspection) -> Result<u64, DebugError> {
    inspection
        .registers()
        .get(PROGRAM_COUNTER_REGISTER)
        .copied()
        .ok_or(DebugError::MissingProgramCounter)
}

fn require_program_counter(inspection: &MachineInspection) -> Result<(), DebugError> {
    program_counter(inspection).map(|_| ())
}

/// Returns one past the next unused id, preferring the lowest free id at or
/// above the stored cursor.
fn next_id<T>(used: &BTreeMap<u32, T>, next: &mut u32) -> u32 {
    let mut candidate = (*next).max(1);
    while used.contains_key(&candidate) {
        candidate = candidate.checked_add(1).unwrap_or(1);
    }
    *next = candidate.checked_add(1).unwrap_or(1);
    candidate
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chronofork_backend_api::MachineInspection;
    use chronofork_core::Tick;

    use super::{MAX_MEMORY_CONDITION_BYTES, next_id, validate_location, validate_log_message};
    use crate::error::{DebugError, InvalidLogMessage};
    use crate::trigger::Location;

    fn inspection(registers: &[(&str, u64)]) -> MachineInspection {
        MachineInspection {
            tick: Tick::ZERO,
            halted: false,
            frame_index: 0,
            cpu_registers: registers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect::<BTreeMap<_, _>>(),
            device_registers: BTreeMap::new(),
            memory: None,
        }
    }

    #[test]
    fn location_validation_rejects_only_inverted_ranges() {
        assert_eq!(validate_location(Location::Address(0x200)), Ok(()));
        assert_eq!(
            validate_location(Location::Range {
                start: 0x200,
                end: 0x200
            }),
            Ok(())
        );
        assert_eq!(
            validate_location(Location::Range {
                start: 0x201,
                end: 0x200
            }),
            Err(DebugError::InvalidRange {
                start: 0x201,
                end: 0x200
            })
        );
    }

    #[test]
    fn log_message_validation_is_bounded() {
        assert_eq!(validate_log_message("store"), Ok(()));
        assert_eq!(validate_log_message(&"x".repeat(128)), Ok(()));
        assert_eq!(
            validate_log_message(""),
            Err(DebugError::InvalidLogMessage(InvalidLogMessage::Empty))
        );
        assert_eq!(
            validate_log_message(&"x".repeat(129)),
            Err(DebugError::InvalidLogMessage(InvalidLogMessage::TooLong {
                length: 129
            }))
        );
        assert_eq!(
            validate_log_message("a\nb"),
            Err(DebugError::InvalidLogMessage(
                InvalidLogMessage::ContainsLineBreak
            ))
        );
        assert_eq!(
            validate_log_message("a\rb"),
            Err(DebugError::InvalidLogMessage(
                InvalidLogMessage::ContainsLineBreak
            ))
        );
        assert_eq!(MAX_MEMORY_CONDITION_BYTES, 8);
        assert_eq!(inspection(&[("pc", 0x200)]).registers()["pc"], 0x200);
    }

    #[test]
    fn next_id_skips_ids_still_in_the_table() {
        let mut used = BTreeMap::new();
        used.insert(1_u32, ());
        used.insert(3_u32, ());
        let mut next = 1;
        assert_eq!(next_id(&used, &mut next), 2);
        assert_eq!(next, 3);
        assert_eq!(next_id(&used, &mut next), 4);
        assert_eq!(next, 5);
    }
}
