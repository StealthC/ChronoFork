//! Bounded, deterministic instruction trace for the 68000 slice.
//!
//! The trace is **debug data, not canonical state**. It records the address and
//! opcode word of each instruction the CPU attempts to execute, in a fixed-capacity
//! ring buffer. It is disabled by default, is not part of `M68000` snapshots, state
//! hashes or backend payloads, and is not restored from a snapshot.
//!
//! # Recording rules
//!
//! One [`TraceEntry`] is appended after the opcode word has been fetched
//! successfully and before it is decoded. An instruction that is recorded and then
//! fails with [`crate::CpuError::IllegalOpcode`] or
//! [`crate::CpuError::Unsupported`] is still in the trace, which is what a
//! differential run needs in order to explain the divergence. A fetch that fails
//! before the opcode word is available records nothing. Interrupt exception entry
//! is not an instruction and is not recorded.
//!
//! `index` is the zero-based position of the instruction in the run since the trace
//! was last enabled, cleared or taken. It keeps counting when the ring overflows,
//! so the oldest retained record's index reveals how many records were evicted.
//! Indexes saturate at [`u64::MAX`] (unreachable in practice) rather than wrapping.
//!
//! # Cost
//!
//! A CPU with tracing disabled stores an `Option` that is always `None`; each step
//! pays one branch on that option. Storage is allocated lazily by the ring and is
//! bounded by the configured capacity.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

/// One executed instruction as recorded by the bounded trace.
///
/// The fields are the record's zero-based instruction index, the address the
/// opcode word was fetched from, and the opcode word itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceEntry {
    index: u64,
    pc: u32,
    opcode: u16,
}

impl TraceEntry {
    const fn new(index: u64, pc: u32, opcode: u16) -> Self {
        Self { index, pc, opcode }
    }

    /// Returns the zero-based instruction index since the trace was last enabled,
    /// cleared or taken.
    #[must_use]
    pub const fn index(self) -> u64 {
        self.index
    }

    /// Returns the address the opcode word was fetched from.
    #[must_use]
    pub const fn pc(self) -> u32 {
        self.pc
    }

    /// Returns the fetched opcode word.
    #[must_use]
    pub const fn opcode(self) -> u16 {
        self.opcode
    }
}

/// A failure while configuring the instruction trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceError {
    /// A ring-buffer capacity of zero was requested.
    ///
    /// Capacity must be at least one record; callers that want no trace should
    /// leave tracing disabled or disable it explicitly instead.
    ZeroCapacity,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("instruction trace capacity must be at least 1"),
        }
    }
}

impl Error for TraceError {}

/// A fixed-capacity ring of executed-instruction records.
///
/// Build with [`InstructionTrace::new`]; the CPU records into it while tracing is
/// enabled. When full, the oldest record is evicted and counted in
/// [`InstructionTrace::dropped`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionTrace {
    capacity: usize,
    entries: VecDeque<TraceEntry>,
    next_index: u64,
    dropped: u64,
}

impl InstructionTrace {
    /// Creates an empty trace with a bounded ring capacity in records.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::ZeroCapacity`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, TraceError> {
        if capacity == 0 {
            return Err(TraceError::ZeroCapacity);
        }
        Ok(Self {
            capacity,
            entries: VecDeque::new(),
            next_index: 0,
            dropped: 0,
        })
    }

    /// Returns the configured ring capacity in records.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of retained records, which never exceeds the capacity.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no records are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns how many of the oldest records were evicted after the ring filled.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Returns the index of the oldest retained record, or `None` when empty.
    ///
    /// A non-zero value proves the ring overflowed.
    #[must_use]
    pub fn first_index(&self) -> Option<u64> {
        self.entries.front().map(|entry| entry.index)
    }

    /// Returns the index the next recorded instruction will receive.
    #[must_use]
    pub const fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Iterates over the retained records from oldest to newest.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = TraceEntry> + ExactSizeIterator + '_ {
        self.entries.iter().copied()
    }

    /// Returns the retained record at `position` (`0` is the oldest), or `None`.
    #[must_use]
    pub fn get(&self, position: usize) -> Option<TraceEntry> {
        self.entries.get(position).copied()
    }

    /// Discards every record and restarts indexes at zero, keeping the capacity.
    ///
    /// This also clears [`InstructionTrace::dropped`] and
    /// [`InstructionTrace::next_index`].
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_index = 0;
        self.dropped = 0;
    }

    /// Appends one record, evicting the oldest when the ring is full.
    pub(crate) fn record(&mut self, pc: u32, opcode: u16) {
        let entry = TraceEntry::new(self.next_index, pc, opcode);
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(entry);
        self.next_index = self.next_index.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{InstructionTrace, TraceEntry, TraceError};

    #[test]
    fn zero_capacity_is_a_typed_error() {
        assert_eq!(InstructionTrace::new(0), Err(TraceError::ZeroCapacity));
    }

    #[test]
    fn ring_evicts_oldest_and_counts_dropped_records() {
        let mut trace = InstructionTrace::new(2).expect("capacity 2");
        for index in 0..4_u16 {
            trace.record(0x200 + u32::from(index) * 2, 0x4E71);
        }
        assert_eq!(trace.capacity(), 2);
        assert_eq!(trace.len(), 2);
        assert_eq!(trace.dropped(), 2);
        assert_eq!(trace.first_index(), Some(2));
        assert_eq!(trace.next_index(), 4);
        assert_eq!(
            trace.entries().collect::<Vec<_>>(),
            vec![
                TraceEntry::new(2, 0x204, 0x4E71),
                TraceEntry::new(3, 0x206, 0x4E71)
            ]
        );
        assert_eq!(trace.get(1), Some(TraceEntry::new(3, 0x206, 0x4E71)));
        assert_eq!(trace.get(2), None);
    }

    #[test]
    fn clear_restarts_indexes_and_keeps_capacity() {
        let mut trace = InstructionTrace::new(3).expect("capacity 3");
        trace.record(0x200, 0x7001);
        trace.record(0x202, 0x7202);
        trace.clear();
        assert!(trace.is_empty());
        assert_eq!(trace.capacity(), 3);
        assert_eq!(trace.dropped(), 0);
        assert_eq!(trace.first_index(), None);
        assert_eq!(trace.next_index(), 0);
        trace.record(0x200, 0x4E71);
        assert_eq!(trace.get(0), Some(TraceEntry::new(0, 0x200, 0x4E71)));
    }
}
