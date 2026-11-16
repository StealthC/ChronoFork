//! Bounded state cache for the in-process rewind player.
//!
//! Implements the cache model of
//! [ADR-0007](../../../docs/adr/0007-in-process-rewind-and-interactive-stepping.md)
//! §3: validated cadence and bounds, full canonical backend states keyed by
//! recorded step index, a deterministic `payload + 256` byte metric and
//! lowest-step-first eviction. [`crate::player::Player`] owns exactly one cache;
//! this module performs no I/O and holds no global state.

use std::collections::BTreeMap;

use chronofork_backend_api::BackendSnapshot;
use chronofork_core::{StateHash, Tick};

use crate::movie::MAX_STEPS;

use super::PlayerError;

/// Default capture cadence in recorded steps.
///
/// A value of `1` captures the state after every recorded step. A value of `N`
/// captures after steps `N-1, 2N-1, …` using the same index convention as the
/// v2 movie anchors.
pub const DEFAULT_CACHE_INTERVAL_STEPS: usize = 1;

/// Default maximum number of cached states (the floor is excluded).
pub const DEFAULT_CACHE_CAPACITY: usize = 256;

/// Hard maximum number of cached states.
pub const MAX_CACHE_CAPACITY: usize = 65_536;

/// Default byte budget for the deterministic cache byte metric.
pub const DEFAULT_CACHE_BYTE_LIMIT: usize = 16 * 1024 * 1024;

/// Hard maximum byte budget for the deterministic cache byte metric.
pub const MAX_CACHE_BYTE_LIMIT: usize = 64 * 1024 * 1024;

/// Deterministic per-entry accounting overhead added to the opaque payload
/// length.
///
/// The value accounts for the canonical envelope fields, the state digest, the
/// tick and the ordered-map node. It is an accounting estimate, not an
/// allocator measurement; reproducibility is what makes it enforceable and
/// reportable.
pub const STATE_CACHE_ENTRY_OVERHEAD: usize = 256;

/// Validated bounds and cadence for one player's in-process state cache.
///
/// Values are accepted only through [`CacheOptions::new`] (or the documented
/// [`Default`], whose constants are valid by construction), so no cache
/// structure is ever sized from an unvalidated caller value. The limits are
/// fixed by ADR-0007 §3/§4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheOptions {
    interval: usize,
    capacity: usize,
    byte_limit: usize,
}

impl CacheOptions {
    /// Validates a cadence and both cache bounds.
    ///
    /// `interval` is the number of recorded steps between captures; `0` disables
    /// capture and a value above the movie step limit
    /// ([`MAX_STEPS`](crate::movie::MAX_STEPS)) is rejected. `capacity` must be
    /// in `1..=MAX_CACHE_CAPACITY` and `byte_limit` in
    /// `1..=MAX_CACHE_BYTE_LIMIT`. Every violation is a typed
    /// [`PlayerError::InvalidCacheConfig`] raised before a session is imported.
    pub fn new(interval: usize, capacity: usize, byte_limit: usize) -> Result<Self, PlayerError> {
        if capacity == 0 {
            return Err(PlayerError::InvalidCacheConfig {
                field: "cache_capacity",
                reason: "must be at least 1".to_owned(),
            });
        }
        if capacity > MAX_CACHE_CAPACITY {
            return Err(PlayerError::InvalidCacheConfig {
                field: "cache_capacity",
                reason: format!("{capacity} exceeds the maximum of {MAX_CACHE_CAPACITY}"),
            });
        }
        if byte_limit == 0 {
            return Err(PlayerError::InvalidCacheConfig {
                field: "cache_bytes",
                reason: "must be at least 1".to_owned(),
            });
        }
        if byte_limit > MAX_CACHE_BYTE_LIMIT {
            return Err(PlayerError::InvalidCacheConfig {
                field: "cache_bytes",
                reason: format!("{byte_limit} exceeds the maximum of {MAX_CACHE_BYTE_LIMIT}"),
            });
        }
        if interval > MAX_STEPS {
            return Err(PlayerError::InvalidCacheConfig {
                field: "cache_interval",
                reason: format!("{interval} exceeds the maximum of {MAX_STEPS}"),
            });
        }
        Ok(Self {
            interval,
            capacity,
            byte_limit,
        })
    }

    /// Returns the capture cadence in recorded steps (`0` disables capture).
    #[must_use]
    pub const fn interval(&self) -> usize {
        self.interval
    }

    /// Returns the maximum number of cached states.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the byte budget for the deterministic byte metric.
    #[must_use]
    pub const fn byte_limit(&self) -> usize {
        self.byte_limit
    }
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            interval: DEFAULT_CACHE_INTERVAL_STEPS,
            capacity: DEFAULT_CACHE_CAPACITY,
            byte_limit: DEFAULT_CACHE_BYTE_LIMIT,
        }
    }
}

/// Configured bounds and current deterministic usage of one player's cache.
///
/// `bytes` is the enforced and reported metric
/// `Σ(payload length + STATE_CACHE_ENTRY_OVERHEAD)` over cached entries; it is
/// reproducible across processes and builds. The floor (the embedded
/// checkpoint) is not an entry and is not counted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheStats {
    /// Configured capture cadence in recorded steps.
    pub interval: usize,
    /// Configured maximum number of entries.
    pub capacity: usize,
    /// Configured byte budget.
    pub byte_limit: usize,
    /// Number of cached states, excluding the floor.
    pub entries: usize,
    /// Deterministic byte metric of the cached states.
    pub bytes: usize,
    /// Number of entries evicted since the player was opened.
    pub evictions: u64,
}

/// One verified canonical state held by the bounded cache.
#[derive(Debug)]
pub(super) struct CachedState {
    pub(super) tick: Tick,
    pub(super) state_hash: StateHash,
    pub(super) state: BackendSnapshot,
    pub(super) bytes: usize,
}

/// The player's private bounded state cache.
///
/// Entries are keyed by the zero-based recorded step whose outcome produced
/// them. The ordered map makes the greatest entry at or before a target an
/// exact `range(..).next_back()` lookup rather than a scan.
#[derive(Debug)]
pub(super) struct StateCache {
    pub(super) entries: BTreeMap<usize, CachedState>,
    pub(super) bytes: usize,
    evictions: u64,
    options: CacheOptions,
}

impl StateCache {
    pub(super) fn new(options: CacheOptions) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            evictions: 0,
            options,
        }
    }

    /// Returns whether the recorded step index belongs to the capture cadence.
    pub(super) fn should_capture(&self, step_index: usize) -> bool {
        self.options.interval != 0
            && step_index
                .checked_add(1)
                .is_some_and(|position| position % self.options.interval == 0)
    }

    pub(super) fn contains(&self, step_index: usize) -> bool {
        self.entries.contains_key(&step_index)
    }

    /// Returns the greatest entry whose recorded step index is below
    /// `target_position`, where a position counts applied steps from the start.
    pub(super) fn select(&self, target_position: usize) -> Option<(usize, &CachedState)> {
        let (&step_index, entry) = self.entries.range(..target_position).next_back()?;
        Some((step_index, entry))
    }

    /// Inserts one verified state, evicting lowest step indices until both
    /// bounds hold. The new entry is inserted last and is never the victim.
    pub(super) fn insert(
        &mut self,
        step_index: usize,
        tick: Tick,
        state_hash: StateHash,
        state: BackendSnapshot,
    ) -> Result<(), PlayerError> {
        let entry_bytes = entry_bytes(&state)?;
        if entry_bytes > self.options.byte_limit {
            return Err(PlayerError::CacheEntryTooLarge {
                bytes: entry_bytes,
                limit: self.options.byte_limit,
            });
        }
        loop {
            let projected =
                self.bytes
                    .checked_add(entry_bytes)
                    .ok_or(PlayerError::ArithmeticOverflow {
                        field: "cache_bytes",
                    })?;
            let count_fits = self.entries.len() < self.options.capacity;
            if count_fits && projected <= self.options.byte_limit {
                break;
            }
            let Some(lowest) = self.entries.keys().next().copied() else {
                break;
            };
            let Some(removed) = self.entries.remove(&lowest) else {
                break;
            };
            self.bytes =
                self.bytes
                    .checked_sub(removed.bytes)
                    .ok_or(PlayerError::ArithmeticOverflow {
                        field: "cache_bytes",
                    })?;
            self.evictions =
                self.evictions
                    .checked_add(1)
                    .ok_or(PlayerError::ArithmeticOverflow {
                        field: "cache_evictions",
                    })?;
        }
        self.bytes =
            self.bytes
                .checked_add(entry_bytes)
                .ok_or(PlayerError::ArithmeticOverflow {
                    field: "cache_bytes",
                })?;
        self.entries.insert(
            step_index,
            CachedState {
                tick,
                state_hash,
                state,
                bytes: entry_bytes,
            },
        );
        Ok(())
    }

    pub(super) fn stats(&self) -> CacheStats {
        CacheStats {
            interval: self.options.interval,
            capacity: self.options.capacity,
            byte_limit: self.options.byte_limit,
            entries: self.entries.len(),
            bytes: self.bytes,
            evictions: self.evictions,
        }
    }
}

/// Returns the deterministic byte metric of one captured state.
fn entry_bytes(state: &BackendSnapshot) -> Result<usize, PlayerError> {
    state
        .payload()
        .len()
        .checked_add(STATE_CACHE_ENTRY_OVERHEAD)
        .ok_or(PlayerError::ArithmeticOverflow {
            field: "cache_entry_bytes",
        })
}
