//! Bounded VDP write FIFO state.
//!
//! The real VDP accepts CPU data-port writes into a small FIFO and stalls the
//! CPU when it is full. Version 2 snapshots the queue because its contents are
//! canonical state: a restore must reproduce the same pending writes in the
//! same order. Phase 8.3 task 9 wires port writes and drain/wait accounting to
//! this type; until then the queue is part of reset and snapshot state only.

/// Number of 16-bit words the VDP write FIFO can hold.
pub const FIFO_CAPACITY: usize = 4;

/// A bounded first-in-first-out queue of 16-bit port writes.
///
/// Only the first [`Fifo::len`] words are meaningful; the remaining slots are
/// zeroed by [`Fifo::words`] so two FIFOs with the same logical contents always
/// encode to the same canonical bytes regardless of their history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Fifo {
    words: [u16; FIFO_CAPACITY],
    head: usize,
    len: usize,
}

impl Fifo {
    /// Creates an empty FIFO.
    pub(super) const fn empty() -> Self {
        Self {
            words: [0; FIFO_CAPACITY],
            head: 0,
            len: 0,
        }
    }

    /// Returns the number of queued words.
    pub(super) const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the queue holds no words.
    pub(super) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns whether the queue holds [`FIFO_CAPACITY`] words.
    pub(super) const fn is_full(&self) -> bool {
        self.len == FIFO_CAPACITY
    }

    /// Returns the queued words oldest first, zero-padded to capacity.
    pub(super) fn words(&self) -> [u16; FIFO_CAPACITY] {
        let mut words = [0_u16; FIFO_CAPACITY];
        for (index, slot) in words.iter_mut().enumerate() {
            if index < self.len {
                *slot = self.words[(self.head + index) % FIFO_CAPACITY];
            }
        }
        words
    }

    /// Rebuilds a FIFO from a restore candidate.
    ///
    /// Returns `None` when `count` exceeds [`FIFO_CAPACITY`]; the caller maps
    /// that to its typed snapshot error. This is the single validity check for
    /// a serialized FIFO occupancy.
    pub(super) fn try_restore(count: usize, words: [u16; FIFO_CAPACITY]) -> Option<Self> {
        if count > FIFO_CAPACITY {
            return None;
        }
        Some(Self {
            words,
            head: 0,
            len: count,
        })
    }
}
