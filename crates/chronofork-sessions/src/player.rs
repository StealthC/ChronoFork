//! Bounded in-process rewind state cache and deterministic rewind player.
//!
//! This module implements the session-layer half of
//! [ADR-0007](../../../docs/adr/0007-in-process-rewind-and-interactive-stepping.md):
//! a [`Player`] that owns a private [`SessionManager`] with exactly one imported
//! session, the decoded [`Movie`], the current verified position and one bounded
//! state cache (see the private `cache` submodule).
//!
//! Rewind is restore-plus-replay, never random access: a seek restores the
//! greatest cached state at or before the target (or the embedded checkpoint at
//! the movie start) and replays the recorded inputs forward, verifying every
//! `(outcome tick, state hash)` expectation and reporting the exact first
//! divergence. Cache entries are full canonical backend states captured only
//! after the producing step was verified and are keyed by recorded step index;
//! both an entry-count bound and a deterministic `payload + 256` byte metric are
//! enforced, and eviction removes the lowest retained step index first.
//!
//! The player performs no file or storage I/O, owns no global or static mutable
//! state and reads no host clock or entropy. A player's private session exposes
//! no checkpoint identity: captures never allocate a snapshot id or insert into
//! the session's snapshot map.
//!
//! Implementation status: added under the approved ADR-0007 contract. It is not
//! independently verified and must not be cited as an accepted capability until
//! the M3 rewind acceptance plan rows execute.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_core::{SessionId, StateHash, Tick};

use crate::error::SessionError;
use crate::manager::SessionManager;
use crate::movie::{Movie, MovieError};

mod cache;

use cache::StateCache;
pub use cache::{
    CacheOptions, CacheStats, DEFAULT_CACHE_BYTE_LIMIT, DEFAULT_CACHE_CAPACITY,
    DEFAULT_CACHE_INTERVAL_STEPS, MAX_CACHE_BYTE_LIMIT, MAX_CACHE_CAPACITY,
    STATE_CACHE_ENTRY_OVERHEAD,
};

/// The verified position of one player: the movie start or one recorded step.
///
/// `applied_steps` is the number of recorded steps applied since the movie
/// start, so the start position is `0` and the position after recorded step `k`
/// is `k + 1`. The tick and state digest are observed values produced by
/// executing or restoring a verified state, never interpolated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerState {
    /// Observed tick at this position.
    pub tick: Tick,
    /// Recorded steps applied since the movie start.
    pub applied_steps: usize,
    /// Observed state digest at this position.
    pub state_hash: StateHash,
}

/// Where a seek started from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartKind {
    /// The embedded checkpoint at the movie start tick, outside the cache.
    Floor,
    /// A verified cached state.
    Cache,
}

impl fmt::Display for StartKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Floor => f.write_str("floor"),
            Self::Cache => f.write_str("cache"),
        }
    }
}

/// Result of one successful seek or forward step.
///
/// `tick` and `state_hash` equal the recorded expectation at `target_tick`;
/// `start`, `start_tick` and `start_step` describe the restored state, and
/// `replayed_steps` is the number of recorded inputs this command executed
/// (zero when the selected start state is already the target position).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeekReport {
    /// Requested and reached recorded position.
    pub target_tick: Tick,
    /// Observed tick at the position (equal to `target_tick` on success).
    pub tick: Tick,
    /// Recorded steps applied since the movie start.
    pub applied_steps: usize,
    /// Whether the seek started from the floor or a cached state.
    pub start: StartKind,
    /// Tick of the chosen start state.
    pub start_tick: Tick,
    /// Recorded step index of the chosen cached state, absent for the floor.
    pub start_step: Option<usize>,
    /// Recorded inputs executed by this command.
    pub replayed_steps: usize,
    /// Observed state digest at the position.
    pub state_hash: StateHash,
    /// Cache configuration and deterministic usage after the command.
    pub cache: CacheStats,
}

/// In-flight seek context carried by a replay failure.
///
/// This lets an interactive adapter report the exact start state, executed-step
/// count and cache metrics of a failed seek without re-deriving the selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeekProgress {
    /// Target that was being sought.
    pub target_tick: Tick,
    /// Chosen start state.
    pub start: StartKind,
    /// Tick of the chosen start state.
    pub start_tick: Tick,
    /// Recorded step index of the chosen cached state, absent for the floor.
    pub start_step: Option<usize>,
    /// Recorded inputs executed by the failed command; a tick or hash mismatch
    /// counts the input that produced it.
    pub replayed_steps: usize,
    /// Cache entries at the moment of failure.
    pub cache_entries: usize,
    /// Deterministic cache bytes at the moment of failure.
    pub cache_bytes: usize,
}

/// A typed failure at the rewind player boundary.
///
/// Backend/session and movie/replay failures preserve their source error; no
/// failure is flattened to a string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlayerError {
    /// The cache cadence or a cache bound is outside its fixed range.
    InvalidCacheConfig {
        /// Configuration field that was rejected.
        field: &'static str,
        /// Human-readable reason.
        reason: String,
    },
    /// A single captured state does not fit the configured byte budget.
    CacheEntryTooLarge {
        /// Deterministic byte metric of the captured entry.
        bytes: usize,
        /// Configured byte budget.
        limit: usize,
    },
    /// Byte, count or step-index arithmetic would overflow.
    ArithmeticOverflow {
        /// Field or total whose arithmetic overflowed.
        field: &'static str,
    },
    /// The session layer rejected an import, restore, capture or step.
    Session(SessionError),
    /// A replay step failed or diverged.
    ///
    /// `source` is the existing typed movie category (`TickMismatch`,
    /// `Divergence`, `ReplayFailed` or range validation), preserved rather than
    /// flattened. `progress` is present for a failure during the replay loop, so
    /// the exact start state and executed-step count remain reportable.
    Movie {
        /// Preserved movie/replay error, boxed to keep the error type compact.
        source: Box<MovieError>,
        /// In-flight seek context when the failure happened while replaying.
        progress: Option<SeekProgress>,
    },
    /// The target tick precedes the movie start tick.
    TargetBeforeStart {
        /// Rejected target.
        tick: Tick,
        /// Movie start tick.
        start_tick: Tick,
    },
    /// The target tick follows the movie end tick.
    TargetAfterEnd {
        /// Rejected target.
        tick: Tick,
        /// Movie end tick.
        end_tick: Tick,
    },
    /// The target tick is neither the start tick nor a recorded outcome tick.
    TargetUnaligned {
        /// Rejected target.
        tick: Tick,
    },
    /// `back` was requested at the start position.
    AlreadyAtStart {
        /// Current tick.
        tick: Tick,
    },
    /// `next` was requested at the end position.
    AlreadyAtEnd {
        /// Current tick.
        tick: Tick,
    },
    /// The imported embedded checkpoint does not match the movie's declared
    /// identity.
    FloorMismatch {
        /// Tick declared by the movie.
        expected_tick: Tick,
        /// Tick observed after import or restore.
        observed_tick: Tick,
        /// State digest declared by the movie.
        expected_hash: StateHash,
        /// State digest observed after import or restore.
        observed_hash: StateHash,
    },
    /// A restored cached state does not match the tick and digest it was
    /// captured with.
    RestoreMismatch {
        /// Recorded step index of the restored entry.
        step_index: usize,
        /// Tick stored with the entry.
        expected_tick: Tick,
        /// Tick observed after restore.
        observed_tick: Tick,
        /// State digest stored with the entry.
        expected_hash: StateHash,
        /// State digest observed after restore.
        observed_hash: StateHash,
    },
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCacheConfig { field, reason } => {
                write!(f, "invalid {field}: {reason}")
            }
            Self::CacheEntryTooLarge { bytes, limit } => {
                write!(f, "cache entry is {bytes} bytes, limit is {limit}")
            }
            Self::ArithmeticOverflow { field } => {
                write!(f, "arithmetic overflow in {field}")
            }
            Self::Session(error) => write!(f, "session error: {error}"),
            Self::Movie { source, .. } => write!(f, "{source}"),
            Self::TargetBeforeStart { tick, start_tick } => {
                write!(
                    f,
                    "target tick {tick} is before movie start tick {start_tick}"
                )
            }
            Self::TargetAfterEnd { tick, end_tick } => {
                write!(f, "target tick {tick} is after movie end tick {end_tick}")
            }
            Self::TargetUnaligned { tick } => write!(
                f,
                "target tick {tick} is not the start tick or a recorded outcome tick"
            ),
            Self::AlreadyAtStart { tick } => {
                write!(f, "already at the start position (tick {tick})")
            }
            Self::AlreadyAtEnd { tick } => {
                write!(f, "already at the end position (tick {tick})")
            }
            Self::FloorMismatch {
                expected_tick,
                observed_tick,
                expected_hash,
                observed_hash,
            } => write!(
                f,
                "embedded checkpoint mismatch: expected tick {expected_tick} state \
                 {expected_hash}, observed tick {observed_tick} state {observed_hash}"
            ),
            Self::RestoreMismatch {
                step_index,
                expected_tick,
                observed_tick,
                expected_hash,
                observed_hash,
            } => write!(
                f,
                "cached state at step {step_index} mismatch: expected tick \
                 {expected_tick} state {expected_hash}, observed tick {observed_tick} \
                 state {observed_hash}"
            ),
        }
    }
}

impl Error for PlayerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Session(error) => Some(error),
            Self::Movie { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Builds in-flight seek context for a replay failure.
fn seek_progress(
    target_tick: Tick,
    start: StartKind,
    start_tick: Tick,
    start_step: Option<usize>,
    replayed_steps: usize,
    cache: &StateCache,
) -> SeekProgress {
    SeekProgress {
        target_tick,
        start,
        start_tick,
        start_step,
        replayed_steps,
        cache_entries: cache.entries.len(),
        cache_bytes: cache.bytes,
    }
}

/// One in-process rewind/stepping player over a decoded movie.
///
/// The player owns a private [`SessionManager`] containing exactly one imported
/// session, the movie and its bounded cache. It never accepts a caller session,
/// never writes storage or a file, and stores no state outside this value:
/// dropping the player drops the session and the cache, and a new process starts
/// empty.
///
/// A seek restores the greatest verified cached state at or before the target
/// (or the floor) and replays forward, verifying every recorded expectation. It
/// reports the exact first mismatch as [`PlayerError::Movie`] and then must be
/// discarded: after a restore or replay failure its session no longer describes
/// its last reported position. Target, boundary and configuration failures are
/// decided before any restore or step, so those leave the player position
/// unchanged.
#[derive(Debug)]
pub struct Player {
    manager: SessionManager,
    session: SessionId,
    movie: Movie,
    position: PlayerState,
    cache: StateCache,
}

impl Player {
    /// Opens a player over `movie` with a private manager built from `factory`.
    ///
    /// The movie's embedded checkpoint is imported through the single existing
    /// snapshot import path, its tick and state digest are verified against the
    /// movie's declared identity, and the imported checkpoint record is dropped:
    /// the floor stays in the session's creation state, outside the cache and
    /// outside the session's checkpoint map.
    pub fn open(
        factory: Arc<dyn BackendFactory>,
        movie: Movie,
        options: CacheOptions,
    ) -> Result<Self, PlayerError> {
        let mut manager = SessionManager::new(factory);
        let session = manager
            .create_session_from_snapshot_bytes(movie.snapshot_bytes())
            .map_err(PlayerError::Session)?;
        manager
            .discard_session_checkpoints(session)
            .map_err(PlayerError::Session)?;
        let inspection = manager.inspect(session).map_err(PlayerError::Session)?;
        let observed_tick = inspection.machine.tick;
        let observed_hash = inspection.machine.state_hash;
        let expected_tick = movie.start_tick();
        let expected_hash = movie.checkpoint_state_hash();
        if observed_tick != expected_tick || observed_hash != expected_hash {
            return Err(PlayerError::FloorMismatch {
                expected_tick,
                observed_tick,
                expected_hash,
                observed_hash,
            });
        }
        Ok(Self {
            manager,
            session,
            movie,
            position: PlayerState {
                tick: observed_tick,
                applied_steps: 0,
                state_hash: observed_hash,
            },
            cache: StateCache::new(options),
        })
    }

    /// Returns the decoded movie this player steps through.
    #[must_use]
    pub const fn movie(&self) -> &Movie {
        &self.movie
    }

    /// Returns the current verified position.
    #[must_use]
    pub const fn state(&self) -> PlayerState {
        self.position
    }

    /// Returns the cache configuration and deterministic usage.
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    /// Rewinds or advances to the exact recorded position at `tick`.
    ///
    /// The target must be the movie start tick or a recorded outcome tick;
    /// anything else is a typed target error before any restore or step. On
    /// success the greatest verified cached state at or before the target (or
    /// the floor) is restored, its stored tick and digest are checked, the
    /// recorded inputs after it are applied once each and verified, and cadence
    /// states are captured. The first mismatch is reported as
    /// [`PlayerError::Movie`] with the exact recorded tick and input index.
    pub fn seek(&mut self, tick: Tick) -> Result<SeekReport, PlayerError> {
        let target_position = self.resolve_target(tick)?;
        self.run_to(target_position, tick)
    }

    /// Advances to the next recorded position.
    ///
    /// The start position counts as a position, so a forward step from the start
    /// seeks to the first recorded step. A forward step at the end is
    /// [`PlayerError::AlreadyAtEnd`] and changes nothing.
    pub fn step_forward(&mut self) -> Result<SeekReport, PlayerError> {
        let Some(step) = self.movie.steps().get(self.position.applied_steps) else {
            return Err(PlayerError::AlreadyAtEnd {
                tick: self.position.tick,
            });
        };
        let tick = step.outcome_tick();
        self.seek(tick)
    }

    /// Steps back to the previous recorded position.
    ///
    /// The start position counts as a position, so a backward step from the first
    /// recorded step seeks to the movie start tick. A backward step at the start is
    /// [`PlayerError::AlreadyAtStart`] and changes nothing.
    pub fn step_back(&mut self) -> Result<SeekReport, PlayerError> {
        let current = self.position.applied_steps;
        if current == 0 {
            return Err(PlayerError::AlreadyAtStart {
                tick: self.position.tick,
            });
        }
        let target_position = current - 1;
        let tick = self.tick_at_position(target_position);
        self.seek(tick)
    }

    /// Resolves an aligned target tick to its applied-step position.
    ///
    /// The check order is before-start, after-end and alignment, so the first
    /// violation in that order is reported. Recorded ticks are strictly
    /// increasing (validated by the movie reader and constructor), so the
    /// alignment lookup is an exact binary search.
    fn resolve_target(&self, tick: Tick) -> Result<usize, PlayerError> {
        let start_tick = self.movie.start_tick();
        if tick < start_tick {
            return Err(PlayerError::TargetBeforeStart { tick, start_tick });
        }
        let end_tick = self.movie.end_tick();
        if tick > end_tick {
            return Err(PlayerError::TargetAfterEnd { tick, end_tick });
        }
        if tick == start_tick {
            return Ok(0);
        }
        match self.movie.recorded_step_index(tick) {
            Some(step_index) => step_index
                .checked_add(1)
                .ok_or(PlayerError::ArithmeticOverflow {
                    field: "step_index",
                }),
            None => Err(PlayerError::TargetUnaligned { tick }),
        }
    }

    /// Returns the recorded tick at an applied-step position.
    fn tick_at_position(&self, position: usize) -> Tick {
        if position == 0 {
            return self.movie.start_tick();
        }
        // The player only stores positions bounded by the validated step count,
        // so the preceding recorded step always exists.
        self.movie.steps()[position - 1].outcome_tick()
    }

    /// Restores the selected start state, replays through the target and
    /// verifies every recorded expectation.
    fn run_to(
        &mut self,
        target_position: usize,
        target_tick: Tick,
    ) -> Result<SeekReport, PlayerError> {
        let (start_kind, start_tick, start_hash, start_position, start_step) =
            match self.cache.select(target_position) {
                Some((step_index, entry)) => {
                    let tick = entry.tick;
                    let state_hash = entry.state_hash;
                    self.manager
                        .restore_session_state(self.session, &entry.state)
                        .map_err(PlayerError::Session)?;
                    let inspection = self
                        .manager
                        .inspect(self.session)
                        .map_err(PlayerError::Session)?;
                    let observed_tick = inspection.machine.tick;
                    let observed_hash = inspection.machine.state_hash;
                    if observed_tick != tick || observed_hash != state_hash {
                        return Err(PlayerError::RestoreMismatch {
                            step_index,
                            expected_tick: tick,
                            observed_tick,
                            expected_hash: state_hash,
                            observed_hash,
                        });
                    }
                    let position =
                        step_index
                            .checked_add(1)
                            .ok_or(PlayerError::ArithmeticOverflow {
                                field: "step_index",
                            })?;
                    (
                        StartKind::Cache,
                        tick,
                        state_hash,
                        position,
                        Some(step_index),
                    )
                }
                None => {
                    self.manager
                        .reset(self.session)
                        .map_err(PlayerError::Session)?;
                    let inspection = self
                        .manager
                        .inspect(self.session)
                        .map_err(PlayerError::Session)?;
                    let observed_tick = inspection.machine.tick;
                    let observed_hash = inspection.machine.state_hash;
                    let expected_tick = self.movie.start_tick();
                    let expected_hash = self.movie.checkpoint_state_hash();
                    if observed_tick != expected_tick || observed_hash != expected_hash {
                        return Err(PlayerError::FloorMismatch {
                            expected_tick,
                            observed_tick,
                            expected_hash,
                            observed_hash,
                        });
                    }
                    (StartKind::Floor, expected_tick, expected_hash, 0, None)
                }
            };

        let Player {
            manager,
            session,
            movie,
            position,
            cache,
        } = self;
        let mut tick = start_tick;
        let mut state_hash = start_hash;
        let mut replayed_steps = 0_usize;
        for (input_index, step) in movie.steps().iter().enumerate() {
            if input_index < start_position {
                continue;
            }
            if input_index >= target_position {
                break;
            }
            let outcome = match manager.step(*session, step.input()) {
                Ok(outcome) => outcome,
                Err(source) => {
                    return Err(PlayerError::Movie {
                        source: Box::new(MovieError::ReplayFailed {
                            input_index,
                            source,
                        }),
                        progress: Some(seek_progress(
                            target_tick,
                            start_kind,
                            start_tick,
                            start_step,
                            replayed_steps,
                            cache,
                        )),
                    });
                }
            };
            if outcome.tick != step.outcome_tick() {
                return Err(PlayerError::Movie {
                    source: Box::new(MovieError::TickMismatch {
                        input_index,
                        expected: step.outcome_tick(),
                        observed: outcome.tick,
                    }),
                    progress: Some(seek_progress(
                        target_tick,
                        start_kind,
                        start_tick,
                        start_step,
                        replayed_steps + 1,
                        cache,
                    )),
                });
            }
            if outcome.state_hash != step.state_hash() {
                return Err(PlayerError::Movie {
                    source: Box::new(MovieError::Divergence {
                        tick: step.outcome_tick(),
                        input_index,
                        expected: step.state_hash(),
                        observed: outcome.state_hash,
                    }),
                    progress: Some(seek_progress(
                        target_tick,
                        start_kind,
                        start_tick,
                        start_step,
                        replayed_steps + 1,
                        cache,
                    )),
                });
            }
            replayed_steps += 1;
            tick = outcome.tick;
            state_hash = outcome.state_hash;
            if cache.should_capture(input_index) && !cache.contains(input_index) {
                let state = manager
                    .capture_session_state(*session)
                    .map_err(PlayerError::Session)?;
                cache.insert(input_index, outcome.tick, outcome.state_hash, state)?;
            }
        }
        *position = PlayerState {
            tick,
            applied_steps: target_position,
            state_hash,
        };
        Ok(SeekReport {
            target_tick,
            tick,
            applied_steps: target_position,
            start: start_kind,
            start_tick,
            start_step,
            replayed_steps,
            state_hash,
            cache: cache.stats(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_core::InputFrame;
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{
        CacheOptions, DEFAULT_CACHE_BYTE_LIMIT, DEFAULT_CACHE_CAPACITY, MAX_CACHE_BYTE_LIMIT,
        MAX_CACHE_CAPACITY, Player, PlayerError, STATE_CACHE_ENTRY_OVERHEAD, StartKind,
    };
    use crate::manager::SessionManager;
    use crate::movie::{MAX_STEPS, Movie, MovieMeta};

    fn recorded_movie(steps: usize) -> Movie {
        let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
        let source = manager.create_session().expect("source session");
        let inputs = (0..steps)
            .map(|index| InputFrame::new(index as u32 % 7))
            .collect::<Vec<_>>();
        let meta = MovieMeta::new(1, None, None).expect("movie metadata");
        Movie::record(&mut manager, source, meta, &inputs).expect("movie records")
    }

    fn open(steps: usize, options: CacheOptions) -> Player {
        Player::open(
            Arc::new(SyntheticBackendFactory::default()),
            recorded_movie(steps),
            options,
        )
        .expect("player opens")
    }

    #[test]
    fn forward_pass_captures_every_step_without_touching_checkpoints() {
        let mut player = open(6, CacheOptions::default());
        let identity = player
            .manager
            .session_snapshot_identity(player.session)
            .expect("session identity");
        assert_eq!(identity.0, 0, "import record is discarded at open");
        for _ in 0..6 {
            player.step_forward().expect("next step");
        }
        assert_eq!(player.cache.entries.len(), 6);
        assert_eq!(player.cache.stats().evictions, 0);
        assert_eq!(
            player
                .manager
                .session_snapshot_identity(player.session)
                .expect("session identity"),
            identity,
            "captures must not allocate snapshot ids or grow the checkpoint map"
        );
        for entry in player.cache.entries.values() {
            assert_eq!(
                entry.bytes,
                entry.state.payload().len() + STATE_CACHE_ENTRY_OVERHEAD
            );
            assert_eq!(entry.state_hash, entry.state.state_hash());
            assert_eq!(entry.tick, entry.state.tick());
        }
        assert_eq!(
            player.cache.stats().bytes,
            player.cache.entries.values().map(|entry| entry.bytes).sum()
        );
    }

    #[test]
    fn cadence_captures_anchor_step_indices() {
        let mut interval_two = open(
            6,
            CacheOptions::new(2, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_BYTE_LIMIT)
                .expect("valid options"),
        );
        for _ in 0..6 {
            interval_two.step_forward().expect("next step");
        }
        let indices = interval_two
            .cache
            .entries
            .keys()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(indices, vec![1, 3, 5]);

        let mut interval_larger = open(
            4,
            CacheOptions::new(9, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_BYTE_LIMIT)
                .expect("valid options"),
        );
        for _ in 0..4 {
            interval_larger.step_forward().expect("next step");
        }
        assert!(interval_larger.cache.entries.is_empty());

        let mut disabled = open(
            4,
            CacheOptions::new(0, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_BYTE_LIMIT)
                .expect("valid options"),
        );
        for _ in 0..4 {
            disabled.step_forward().expect("next step");
        }
        assert!(disabled.cache.entries.is_empty());
    }

    #[test]
    fn capacity_bound_evicts_the_lowest_step_index_first() {
        let mut player = open(
            10,
            CacheOptions::new(1, 3, DEFAULT_CACHE_BYTE_LIMIT).expect("valid options"),
        );
        for _ in 0..10 {
            player.step_forward().expect("next step");
        }
        let indices = player.cache.entries.keys().copied().collect::<Vec<_>>();
        assert_eq!(indices, vec![7, 8, 9]);
        let stats = player.cache.stats();
        assert_eq!(stats.entries, 3);
        assert_eq!(stats.evictions, 7);
        assert_eq!(
            stats.bytes,
            player.cache.entries.values().map(|entry| entry.bytes).sum()
        );
    }

    #[test]
    fn byte_bound_evicts_the_lowest_step_index_first() {
        let mut probe = open(1, CacheOptions::default());
        probe.step_forward().expect("probe step");
        let entry_bytes = probe.cache.stats().bytes;
        assert!(entry_bytes > 0);
        let limit = entry_bytes * 2;

        let mut player = open(
            6,
            CacheOptions::new(1, MAX_CACHE_CAPACITY, limit).expect("valid options"),
        );
        for _ in 0..6 {
            player.step_forward().expect("next step");
        }
        let stats = player.cache.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.bytes, limit);
        assert_eq!(stats.evictions, 4);
        let indices = player.cache.entries.keys().copied().collect::<Vec<_>>();
        assert_eq!(indices, vec![4, 5]);
    }

    #[test]
    fn a_state_larger_than_the_byte_budget_is_a_typed_error() {
        let mut player = open(
            2,
            CacheOptions::new(1, DEFAULT_CACHE_CAPACITY, 1).expect("valid options"),
        );
        let error = player.step_forward().expect_err("entry cannot fit");
        assert!(
            matches!(
                error,
                PlayerError::CacheEntryTooLarge { bytes, limit }
                    if bytes > 1 && limit == 1
            ),
            "expected cache-entry-too-large, got {error}"
        );
        assert!(player.cache.entries.is_empty());
    }

    #[test]
    fn capture_restore_round_trips_a_cached_state() {
        let mut player = open(3, CacheOptions::default());
        player.step_forward().expect("first step");
        player.step_forward().expect("second step");
        let entry = player.cache.entries.get(&1).expect("second step cached");
        let expected_tick = entry.tick;
        let expected_hash = entry.state_hash;
        let state = entry.state.clone();
        player
            .manager
            .restore_session_state(player.session, &state)
            .expect("restore cached state");
        let inspection = player.manager.inspect(player.session).expect("inspect");
        assert_eq!(inspection.machine.tick, expected_tick);
        assert_eq!(inspection.machine.state_hash, expected_hash);
    }

    #[test]
    fn seeking_to_a_cached_tick_restores_without_replaying() {
        let mut player = open(5, CacheOptions::default());
        for _ in 0..5 {
            player.step_forward().expect("next step");
        }
        let report = player
            .seek(player.movie().start_tick())
            .expect("seek start");
        assert_eq!(report.start, StartKind::Floor);
        assert_eq!(report.start_step, None);
        assert_eq!(report.applied_steps, 0);
        assert_eq!(report.replayed_steps, 0);
        let report = player.seek(player.movie().end_tick()).expect("seek end");
        assert_eq!(report.start, StartKind::Cache);
        assert_eq!(report.start_step, Some(4));
        assert_eq!(report.applied_steps, 5);
        assert_eq!(report.replayed_steps, 0);
    }

    #[test]
    fn cache_configuration_outside_the_fixed_bounds_is_rejected() {
        assert!(CacheOptions::new(0, 1, 1).is_ok());
        assert!(matches!(
            CacheOptions::new(1, 0, 1),
            Err(PlayerError::InvalidCacheConfig {
                field: "cache_capacity",
                ..
            })
        ));
        assert!(matches!(
            CacheOptions::new(1, MAX_CACHE_CAPACITY + 1, 1),
            Err(PlayerError::InvalidCacheConfig {
                field: "cache_capacity",
                ..
            })
        ));
        assert!(matches!(
            CacheOptions::new(1, 1, 0),
            Err(PlayerError::InvalidCacheConfig {
                field: "cache_bytes",
                ..
            })
        ));
        assert!(matches!(
            CacheOptions::new(1, 1, MAX_CACHE_BYTE_LIMIT + 1),
            Err(PlayerError::InvalidCacheConfig {
                field: "cache_bytes",
                ..
            })
        ));
        assert!(matches!(
            CacheOptions::new(MAX_STEPS + 1, 1, 1),
            Err(PlayerError::InvalidCacheConfig {
                field: "cache_interval",
                ..
            })
        ));
    }
}
