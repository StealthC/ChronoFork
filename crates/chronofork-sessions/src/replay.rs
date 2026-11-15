//! Deterministic ranged replay and branch comparison over decoded v1 movies.
//!
//! This module implements the increment-2 contract of
//! [ADR-0005](../../../docs/adr/0005-partial-replay-and-branch-comparison.md),
//! which extends the accepted v1 movie format of
//! [ADR-0004](../../../docs/adr/0004-tas-movie-format.md) without changing a
//! byte of it. [`crate::movie`] remains the single `.cfmovie` reader/writer,
//! recorder and validating constructor; this module owns the tick range, its
//! validation, the single deterministic replay path and the recorded-expectation
//! comparison.
//!
//! A replay always imports the movie's embedded checkpoint into a fresh session
//! through the existing [`SessionManager::create_session_from_snapshot_bytes`]
//! path, so backend identity, payload version and configuration compatibility
//! stay decided by the existing snapshot envelope predicate. Execution then
//! starts at the embedded checkpoint and applies recorded inputs in order; no
//! state is carried between invocations, nothing is cached across calls and a
//! pre-existing session is never touched. Seek is therefore deterministic
//! re-simulation from the only anchor v1 stores, never random access.
//!
//! A [`ReplayRange`] is the pair `(from, until]`: the window contains the
//! recorded steps with `from < outcome_tick <= until`, while the prefix contains
//! the steps with `outcome_tick <= from`. Prefix steps are executed and
//! verified exactly like full-range replay, so a range narrows what is reported
//! and bounds how far execution runs; it never weakens verification. Boundaries
//! must be the movie start tick, its end tick or a recorded outcome tick;
//! anything else is rejected with a typed [`MovieError`] before a session is
//! imported.
//!
//! [`compare_movies`] is a pure value operation over two already-decoded movies:
//! it imports no checkpoint, executes no step and never re-simulates. It reports
//! embedded-checkpoint identity, the first `(outcome tick, state hash)`
//! divergence and a length mismatch separately. Movie metadata and input masks
//! never decide equality. Comparison is not verification; [`Movie::verify`] and
//! [`ReplayRange::replay`] remain the re-simulation paths.
//!
//! Deferred and not implemented here: checkpoint anchors inside the container, a
//! format v2, an in-process rewind/state cache, interactive stepping, rerecord
//! and branch editing, multi-controller/ports/analog/keyboard input,
//! real-hardware input TAS and any TAS import/export adapter.

use chronofork_core::{StateHash, Tick};

use crate::manager::SessionManager;
use crate::movie::{Movie, MovieError};

/// A replay window validated against one decoded movie.
///
/// The range borrows the movie it was validated against, so it cannot be
/// applied to a different movie and its boundaries are never reinterpreted.
/// [`ReplayRange::new`] is the single validating constructor; it accepts only
/// the movie start tick, its end tick or a recorded outcome tick, rejects
/// out-of-range, inverted, unaligned and empty windows with a typed
/// [`MovieError`], and performs no session import, backend step or allocation.
#[derive(Clone, Copy, Debug)]
pub struct ReplayRange<'a> {
    movie: &'a Movie,
    from: Tick,
    until: Tick,
}

impl<'a> ReplayRange<'a> {
    /// Validates a tick window against `movie`.
    ///
    /// An omitted `from` defaults to the movie's start tick and an omitted
    /// `until` to its end tick, so the default request is always the closed full
    /// range `[start_tick, end_tick]`. The `until` boundary is inclusive: steps
    /// at or before `from` are the verified prefix and the recorded step at
    /// `until` is the last window step. Supplied boundaries are never rounded,
    /// clamped or widened: they must be the start tick, the end tick or a
    /// recorded outcome tick, and `from` must not exceed `until`. A window that
    /// contains no recorded step (`from == until` on a movie with at least one
    /// step) is rejected with [`MovieError::RangeEmpty`].
    pub fn new(
        movie: &'a Movie,
        from: Option<Tick>,
        until: Option<Tick>,
    ) -> Result<Self, MovieError> {
        let from = from.unwrap_or_else(|| movie.start_tick());
        let until = until.unwrap_or_else(|| movie.end_tick());
        validate_range(movie, from, until)
    }

    /// Returns the full range `[start_tick, end_tick]` of `movie`.
    ///
    /// The full range of an already validated movie is valid by construction:
    /// the start tick never exceeds the end tick, both are aligned by
    /// definition, and a zero-step movie's only valid window is
    /// `[start_tick, start_tick]`. This is the range [`Movie::verify`] uses, so
    /// full replay goes through the same path as a partial one.
    #[must_use]
    pub fn full(movie: &'a Movie) -> Self {
        Self {
            movie,
            from: movie.start_tick(),
            until: movie.end_tick(),
        }
    }

    /// Replays the validated window from the movie's embedded checkpoint.
    ///
    /// The embedded checkpoint is imported into exactly one fresh session; the
    /// recorded inputs are applied in order up to the step whose recorded
    /// outcome tick is the `until` boundary, and every executed step is compared
    /// with its recorded `(outcome tick, state hash)` exactly as a full replay
    /// does. The first mismatch is reported as [`MovieError::TickMismatch`] or
    /// [`MovieError::Divergence`] with its exact recorded tick and input index;
    /// replay stops there and never resynchronizes. Steps at or before `from`
    /// are counted as the verified prefix, later steps as the window.
    pub fn replay(&self, manager: &mut SessionManager) -> Result<ReplayOutcome, MovieError> {
        replay_validated(self.movie, manager, self.from, self.until)
    }
}

/// Validates a requested window against a decoded movie.
///
/// The check order is bounds (before-start, after-end), inversion, alignment and
/// finally emptiness, so the first violation in that order is reported. It is
/// pure arithmetic over already-decoded ticks: no allocation, no checkpoint
/// decode, no file I/O and no backend execution.
fn validate_range(movie: &Movie, from: Tick, until: Tick) -> Result<ReplayRange<'_>, MovieError> {
    let start_tick = movie.start_tick();
    if from < start_tick {
        return Err(MovieError::RangeBeforeStart { from, start_tick });
    }
    let end_tick = movie.end_tick();
    if until > end_tick {
        return Err(MovieError::RangeAfterEnd { until, end_tick });
    }
    if from > until {
        return Err(MovieError::RangeInverted { from, until });
    }
    if from != start_tick && !is_recorded_tick(movie, from) {
        return Err(MovieError::RangeFromUnaligned { tick: from });
    }
    if until != start_tick && until != end_tick && !is_recorded_tick(movie, until) {
        return Err(MovieError::RangeUntilUnaligned { tick: until });
    }
    if from == until && movie.step_count() != 0 {
        return Err(MovieError::RangeEmpty { from, until });
    }
    Ok(ReplayRange { movie, from, until })
}

/// Returns whether `tick` is one of the movie's recorded outcome ticks.
///
/// Recorded ticks are strictly increasing (already validated by the reader and
/// the validating constructor), so a binary search is exact and allocation-free.
fn is_recorded_tick(movie: &Movie, tick: Tick) -> bool {
    movie
        .steps()
        .binary_search_by_key(&tick.raw(), |step| step.outcome_tick().raw())
        .is_ok()
}

/// Executes one validated window against a fresh imported session.
fn replay_validated(
    movie: &Movie,
    manager: &mut SessionManager,
    from: Tick,
    until: Tick,
) -> Result<ReplayOutcome, MovieError> {
    let session = manager
        .create_session_from_snapshot_bytes(movie.snapshot_bytes())
        .map_err(MovieError::SnapshotImport)?;
    let mut final_tick = movie.start_tick();
    let mut final_state_hash = movie.checkpoint_state_hash();
    let mut prefix_steps = 0_usize;
    let mut window_steps = 0_usize;
    for (input_index, step) in movie.steps().iter().enumerate() {
        if step.outcome_tick() > until {
            break;
        }
        let outcome =
            manager
                .step(session, step.input())
                .map_err(|source| MovieError::ReplayFailed {
                    input_index,
                    source,
                })?;
        if outcome.tick != step.outcome_tick() {
            return Err(MovieError::TickMismatch {
                input_index,
                expected: step.outcome_tick(),
                observed: outcome.tick,
            });
        }
        if outcome.state_hash != step.state_hash() {
            return Err(MovieError::Divergence {
                tick: step.outcome_tick(),
                input_index,
                expected: step.state_hash(),
                observed: outcome.state_hash,
            });
        }
        final_tick = outcome.tick;
        final_state_hash = outcome.state_hash;
        if step.outcome_tick() <= from {
            prefix_steps += 1;
        } else {
            window_steps += 1;
        }
    }
    Ok(ReplayOutcome {
        start_tick: movie.start_tick(),
        end_tick: movie.end_tick(),
        total_steps: movie.step_count(),
        range_from: from,
        range_until: until,
        prefix_steps,
        window_steps,
        replayed_steps: prefix_steps + window_steps,
        final_tick,
        final_state_hash,
    })
}

/// Result of one clean replay over a validated window.
///
/// Every tick and state digest is an observed value that was executed and
/// verified, never an interpolation. For a clean replay `final_tick` equals
/// `range_until` and `final_state_hash` equals the recorded hash of the step at
/// that boundary; for a zero-step movie both describe the embedded checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayOutcome {
    /// Tick of the embedded checkpoint (movie-level fact).
    pub start_tick: Tick,
    /// Last recorded outcome tick, or `start_tick` for a zero-step movie.
    pub end_tick: Tick,
    /// Total recorded step count of the movie.
    pub total_steps: usize,
    /// Validated window start boundary.
    pub range_from: Tick,
    /// Validated window end boundary.
    pub range_until: Tick,
    /// Steps executed before the window (`outcome_tick <= range_from`).
    pub prefix_steps: usize,
    /// Steps in the window (`range_from < outcome_tick <= range_until`).
    pub window_steps: usize,
    /// Steps actually executed and verified (`prefix_steps + window_steps`).
    pub replayed_steps: usize,
    /// Observed tick at `range_until`.
    pub final_tick: Tick,
    /// Observed state digest at `range_until`.
    pub final_state_hash: StateHash,
}

/// Comparison of two decoded movies' recorded expectation records.
///
/// Both movies' checkpoint identity and step counts are always present, so a
/// mismatch verdict can name both sides. The verdict carries the specific
/// evidence of the outcome that was reached. Metadata (`movie_id`, `name`,
/// `parent_movie_id`) and input masks never decide equality and are not part of
/// this report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonReport {
    /// Outcome of the comparison.
    pub verdict: ComparisonVerdict,
    /// First movie's embedded checkpoint start tick.
    pub start_tick_a: Tick,
    /// Second movie's embedded checkpoint start tick.
    pub start_tick_b: Tick,
    /// First movie's embedded checkpoint state digest.
    pub checkpoint_state_hash_a: StateHash,
    /// Second movie's embedded checkpoint state digest.
    pub checkpoint_state_hash_b: StateHash,
    /// First movie's recorded step count.
    pub steps_a: usize,
    /// Second movie's recorded step count.
    pub steps_b: usize,
}

/// Which comparison outcome was reached, in the documented check order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparisonVerdict {
    /// Both checkpoints have the same identity, every recorded step is equal and
    /// the step counts match.
    Identical,
    /// The embedded checkpoint identity (start tick or checkpoint state digest)
    /// differs; no step comparison is attempted.
    CheckpointMismatch,
    /// The first index whose recorded `(outcome tick, state hash)` differs.
    StepDivergence {
        /// Zero-based step index of the first divergence.
        index: usize,
        /// First movie's recorded outcome tick at that index.
        tick_a: Tick,
        /// Second movie's recorded outcome tick at that index.
        tick_b: Tick,
        /// First movie's recorded state digest at that index.
        state_hash_a: StateHash,
        /// Second movie's recorded state digest at that index.
        state_hash_b: StateHash,
    },
    /// The common recorded prefix is equal but the step counts differ.
    LengthMismatch {
        /// Number of equal recorded steps both movies share.
        common_steps: usize,
    },
}

/// Compares two decoded movies' recorded expectations without re-simulating.
///
/// The embedded checkpoint identity (start tick and checkpoint state digest) is
/// compared first: a difference is [`ComparisonVerdict::CheckpointMismatch`] and
/// no step index from different starting states is compared. Next the common
/// step range is walked in ascending order and the first index whose recorded
/// `(outcome tick, state hash)` differs is reported as
/// [`ComparisonVerdict::StepDivergence`]; no later index is reported instead.
/// Finally, an equal common prefix with different step counts is
/// [`ComparisonVerdict::LengthMismatch`]. A length difference never replaces a
/// first divergence.
///
/// This is a pure value operation: it takes only decoded movies, imports no
/// checkpoint, decides no backend compatibility, calls no backend and executes
/// no step. It therefore also compares movies whose embedded checkpoint this
/// build would reject at replay import. A comparison PASS is not a determinism
/// or compatibility proof; [`Movie::verify`] remains the re-simulation path.
#[must_use]
pub fn compare_movies(left: &Movie, right: &Movie) -> ComparisonReport {
    let mut report = ComparisonReport {
        verdict: ComparisonVerdict::Identical,
        start_tick_a: left.start_tick(),
        start_tick_b: right.start_tick(),
        checkpoint_state_hash_a: left.checkpoint_state_hash(),
        checkpoint_state_hash_b: right.checkpoint_state_hash(),
        steps_a: left.step_count(),
        steps_b: right.step_count(),
    };
    if report.start_tick_a != report.start_tick_b
        || report.checkpoint_state_hash_a != report.checkpoint_state_hash_b
    {
        report.verdict = ComparisonVerdict::CheckpointMismatch;
        return report;
    }
    for (index, (step_a, step_b)) in left.steps().iter().zip(right.steps()).enumerate() {
        if step_a.outcome_tick() != step_b.outcome_tick()
            || step_a.state_hash() != step_b.state_hash()
        {
            report.verdict = ComparisonVerdict::StepDivergence {
                index,
                tick_a: step_a.outcome_tick(),
                tick_b: step_b.outcome_tick(),
                state_hash_a: step_a.state_hash(),
                state_hash_b: step_b.state_hash(),
            };
            return report;
        }
    }
    if report.steps_a != report.steps_b {
        report.verdict = ComparisonVerdict::LengthMismatch {
            common_steps: report.steps_a.min(report.steps_b),
        };
    }
    report
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::BackendFactory;
    use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick};
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{ComparisonVerdict, ReplayRange, compare_movies};
    use crate::manager::SessionManager;
    use crate::movie::{Movie, MovieError, MovieMeta, MovieStep};
    use crate::snapshot::Snapshot;

    fn checkpoint_bytes() -> Vec<u8> {
        let backend = SyntheticBackendFactory::default()
            .create()
            .expect("synthetic backend");
        let snapshot = backend.snapshot().expect("synthetic checkpoint");
        Snapshot::new(SnapshotId::from_raw(1), SessionId::from_raw(1), snapshot)
            .to_bytes()
            .expect("checkpoint bytes")
    }

    /// Builds a structurally valid movie with deliberately non-contiguous ticks,
    /// so unaligned boundaries exist and can be exercised.
    fn fabricated_movie(movie_id: u64, ticks: &[u64]) -> Movie {
        let steps = ticks
            .iter()
            .map(|tick| {
                MovieStep::new(
                    Tick::from_raw(*tick),
                    InputFrame::new(1),
                    StateHash::from_bytes([1; 32]),
                )
            })
            .collect();
        Movie::from_parts(
            MovieMeta::new(movie_id, None, None).expect("movie metadata"),
            checkpoint_bytes(),
            steps,
        )
        .expect("fabricated movie")
    }

    fn recorded_movie(source_steps: usize, inputs: &[InputFrame]) -> Movie {
        let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
        let source = manager.create_session().expect("source session");
        for _ in 0..source_steps {
            manager
                .step(source, InputFrame::new(3))
                .expect("advance source");
        }
        Movie::record(
            &mut manager,
            source,
            MovieMeta::new(1, Some("replay-unit".to_owned()), None).expect("movie metadata"),
            inputs,
        )
        .expect("record movie")
    }

    #[test]
    fn range_validation_is_ordered_typed_and_allocation_free()
    -> Result<(), Box<dyn std::error::Error>> {
        let fabricated = fabricated_movie(1, &[5, 8, 12]);
        assert!(ReplayRange::new(&fabricated, None, None).is_ok());
        assert!(ReplayRange::new(&fabricated, Some(Tick::from_raw(5)), None).is_ok());
        assert!(
            ReplayRange::new(
                &fabricated,
                Some(Tick::from_raw(5)),
                Some(Tick::from_raw(8))
            )
            .is_ok()
        );

        // Boundary checks come before inversion, then alignment, then emptiness.
        assert!(matches!(
            ReplayRange::new(&fabricated, Some(Tick::from_raw(13)), None),
            Err(MovieError::RangeInverted { from, until })
                if from == Tick::from_raw(13) && until == Tick::from_raw(12)
        ));
        assert!(matches!(
            ReplayRange::new(&fabricated, None, Some(Tick::from_raw(13))),
            Err(MovieError::RangeAfterEnd { until, end_tick })
                if until == Tick::from_raw(13) && end_tick == Tick::from_raw(12)
        ));
        assert!(matches!(
            ReplayRange::new(&fabricated, Some(Tick::from_raw(6)), None),
            Err(MovieError::RangeFromUnaligned { tick }) if tick == Tick::from_raw(6)
        ));
        assert!(matches!(
            ReplayRange::new(&fabricated, None, Some(Tick::from_raw(6))),
            Err(MovieError::RangeUntilUnaligned { tick }) if tick == Tick::from_raw(6)
        ));
        assert!(matches!(
            ReplayRange::new(&fabricated, Some(Tick::from_raw(8)), Some(Tick::from_raw(8))),
            Err(MovieError::RangeEmpty { from, until })
                if from == Tick::from_raw(8) && until == Tick::from_raw(8)
        ));
        // An explicit start boundary for `until` is aligned, so the equal
        // boundaries are an empty window rather than an unaligned one.
        assert!(matches!(
            ReplayRange::new(&fabricated, None, Some(Tick::ZERO)),
            Err(MovieError::RangeEmpty { .. })
        ));
        // Arbitrary 64-bit ticks never wrap or panic.
        assert!(matches!(
            ReplayRange::new(&fabricated, Some(Tick::from_raw(u64::MAX)), None),
            Err(MovieError::RangeInverted { .. })
        ));
        assert!(matches!(
            ReplayRange::new(&fabricated, None, Some(Tick::from_raw(u64::MAX))),
            Err(MovieError::RangeAfterEnd { .. })
        ));

        let recorded = recorded_movie(3, &[]);
        let before_start = ReplayRange::new(&recorded, Some(Tick::from_raw(2)), None)
            .expect_err("a start before the checkpoint must be rejected");
        assert!(matches!(
            before_start,
            MovieError::RangeBeforeStart { from, start_tick }
                if from == Tick::from_raw(2) && start_tick == Tick::from_raw(3)
        ));
        // Display text keeps both ticks, so a user can act on the rejection.
        let message = before_start.to_string();
        assert!(
            message.contains("2") && message.contains("3"),
            "range error must name both ticks, got {message}"
        );
        Ok(())
    }

    #[test]
    fn comparison_checks_checkpoint_then_divergence_then_length()
    -> Result<(), Box<dyn std::error::Error>> {
        let base = fabricated_movie(1, &[5, 8, 12]);
        let metadata_only = Movie::from_parts(
            MovieMeta::new(2, Some("other".to_owned()), None).expect("metadata"),
            base.snapshot_bytes().to_vec(),
            base.steps().to_vec(),
        )
        .expect("metadata-only movie");
        assert_eq!(
            compare_movies(&base, &metadata_only).verdict,
            ComparisonVerdict::Identical
        );

        let mut altered_steps = base.steps().to_vec();
        let original = altered_steps[1].state_hash();
        altered_steps[1] = MovieStep::new(
            altered_steps[1].outcome_tick(),
            altered_steps[1].input(),
            StateHash::from_bytes([2; 32]),
        );
        let hash_divergent = Movie::from_parts(
            base.meta().clone(),
            base.snapshot_bytes().to_vec(),
            altered_steps,
        )
        .expect("hash-divergent movie");
        match compare_movies(&base, &hash_divergent).verdict {
            ComparisonVerdict::StepDivergence {
                index,
                tick_a,
                tick_b,
                state_hash_a,
                state_hash_b,
            } => {
                assert_eq!(index, 1);
                assert_eq!(tick_a, Tick::from_raw(8));
                assert_eq!(tick_b, Tick::from_raw(8));
                assert_eq!(state_hash_a, original);
                assert_ne!(state_hash_a, state_hash_b);
            }
            other => {
                return Err(format!("expected a step divergence, got {other:?}").into());
            }
        }

        let tick_divergent = fabricated_movie(1, &[5, 9, 12]);
        match compare_movies(&base, &tick_divergent).verdict {
            ComparisonVerdict::StepDivergence {
                index,
                tick_a,
                tick_b,
                state_hash_a,
                state_hash_b,
            } => {
                assert_eq!(index, 1);
                assert_eq!(tick_a, Tick::from_raw(8));
                assert_eq!(tick_b, Tick::from_raw(9));
                assert_eq!(state_hash_a, state_hash_b);
            }
            other => {
                return Err(format!("expected a tick divergence, got {other:?}").into());
            }
        }

        // A divergence in the common prefix wins over the length difference, and
        // both step counts stay visible in the report.
        let shorter_divergent = fabricated_movie(1, &[5, 9]);
        let report = compare_movies(&base, &shorter_divergent);
        assert!(matches!(
            report.verdict,
            ComparisonVerdict::StepDivergence { index: 1, .. }
        ));
        assert_eq!(report.steps_a, 3);
        assert_eq!(report.steps_b, 2);

        let shorter = fabricated_movie(1, &[5, 8]);
        let report = compare_movies(&base, &shorter);
        assert_eq!(
            report.verdict,
            ComparisonVerdict::LengthMismatch { common_steps: 2 }
        );
        assert_eq!(report.steps_a, 3);
        assert_eq!(report.steps_b, 2);

        let other_checkpoint = recorded_movie(3, &[]);
        let report = compare_movies(&base, &other_checkpoint);
        assert_eq!(report.verdict, ComparisonVerdict::CheckpointMismatch);
        assert_ne!(report.start_tick_a, report.start_tick_b);
        Ok(())
    }
}
