//! Ranged replay, typed range rejection and recorded-expectation comparison.
//!
//! These tests exercise the increment-2 library contract on the deterministic
//! synthetic backend, at the sessions layer: replay-to-boundary prefix equality,
//! window/prefix reporting, forward single-step advance and rewind as new
//! invocations, reject-before-import range validation, v1 byte compatibility and
//! branch comparison without re-simulation. They return `Result` instead of
//! unwrapping so the workspace `expect_used` lint stays satisfied without a new
//! file-level allowance.

use std::error::Error;
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId, StateHash, Tick, hash_bytes};
use chronofork_sessions::{
    ComparisonVerdict, MOVIE_FORMAT_VERSION, MOVIE_INPUT_SCHEMA_VERSION, Movie, MovieError,
    MovieMeta, MovieStep, ReplayOutcome, ReplayRange, SessionError, SessionManager, Snapshot,
    compare_movies,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

type TestResult = Result<(), Box<dyn Error>>;

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn meta(movie_id: u64, name: Option<&str>) -> Result<MovieMeta, MovieError> {
    MovieMeta::new(movie_id, name.map(str::to_owned), None)
}

fn recorded_inputs() -> Vec<InputFrame> {
    [0_u32, 1, 7, 3, 0, 255, 16, 9, 4, 2, 8, 5]
        .into_iter()
        .map(InputFrame::new)
        .collect()
}

fn step_many(
    manager: &mut SessionManager,
    session: SessionId,
    input: u32,
    count: usize,
) -> Result<(), SessionError> {
    for _ in 0..count {
        manager.step(session, InputFrame::new(input))?;
    }
    Ok(())
}

fn machine(
    manager: &SessionManager,
    session: SessionId,
) -> Result<(Tick, StateHash), SessionError> {
    let inspection = manager.inspect(session)?;
    Ok((inspection.machine.tick, inspection.machine.state_hash))
}

/// Records `inputs` from a fresh session advanced by `source_steps` ticks.
fn record_from(source_steps: usize, inputs: &[InputFrame]) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 3, source_steps)?;
    Ok(Movie::record(
        &mut manager,
        source,
        meta(1, Some("range"))?,
        inputs,
    )?)
}

/// Rebuilds a movie with the same checkpoint and metadata but different steps.
fn movie_with_steps(movie: &Movie, steps: Vec<MovieStep>) -> Result<Movie, MovieError> {
    Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)
}

fn flip_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0xff;
    StateHash::from_bytes(bytes)
}

/// A structurally valid movie whose recorded ticks have gaps, so that ticks
/// which are neither the start tick nor a recorded outcome tick exist.
fn gapped_movie(base: &Movie, ticks: [u64; 3]) -> Result<Movie, MovieError> {
    let steps = ticks
        .into_iter()
        .enumerate()
        .map(|(index, tick)| {
            MovieStep::new(
                Tick::from_raw(tick),
                InputFrame::new(index as u32),
                StateHash::from_bytes([index as u8 + 1; 32]),
            )
        })
        .collect();
    movie_with_steps(base, steps)
}

#[test]
fn full_range_replay_preserves_the_verify_report() -> TestResult {
    // A multi-step movie with a non-zero start tick: the embedded checkpoint
    // was captured after source activity, so full replay starts there.
    let movie = record_from(3, &recorded_inputs())?;
    let mut replay_manager = new_manager();
    let outcome = ReplayRange::full(&movie).replay(&mut replay_manager)?;
    let report = movie.verify(&mut new_manager())?;

    assert_eq!(outcome.start_tick, report.start_tick);
    assert_eq!(outcome.start_tick, movie.start_tick());
    assert_eq!(outcome.end_tick, report.end_tick);
    assert_eq!(outcome.end_tick, movie.end_tick());
    assert_eq!(outcome.total_steps, movie.step_count());
    assert_eq!(outcome.range_from, movie.start_tick());
    assert_eq!(outcome.range_until, movie.end_tick());
    assert_eq!(outcome.prefix_steps, 0);
    assert_eq!(outcome.window_steps, report.steps);
    assert_eq!(outcome.window_steps, movie.step_count());
    assert_eq!(outcome.replayed_steps, movie.step_count());
    assert_eq!(outcome.final_tick, report.final_tick);
    assert_eq!(outcome.final_tick, movie.end_tick());
    assert_eq!(outcome.final_state_hash, report.final_state_hash);
    assert_eq!(outcome.final_state_hash, movie.final_state_hash());
    // Exactly one fresh imported session, and rerunning is identical.
    assert_eq!(replay_manager.len(), 1);
    assert_eq!(
        ReplayRange::full(&movie).replay(&mut new_manager())?,
        outcome
    );

    // The explicit full boundaries and the default/omitted boundaries agree.
    let explicit = ReplayRange::new(&movie, Some(movie.start_tick()), Some(movie.end_tick()))?;
    assert_eq!(explicit.replay(&mut new_manager())?, outcome);

    // One-step and zero-step movies preserve the same report relationship.
    for (source_steps, inputs) in [(0_usize, vec![InputFrame::new(5)]), (4, Vec::new())] {
        let movie = record_from(source_steps, &inputs)?;
        let outcome = ReplayRange::full(&movie).replay(&mut new_manager())?;
        let report = movie.verify(&mut new_manager())?;
        assert_eq!(outcome.window_steps, report.steps);
        assert_eq!(outcome.window_steps, movie.step_count());
        assert_eq!(outcome.replayed_steps, movie.step_count());
        assert_eq!(outcome.final_tick, report.final_tick);
        assert_eq!(outcome.final_state_hash, report.final_state_hash);
        assert_eq!(outcome.final_state_hash, movie.final_state_hash());
    }
    Ok(())
}

#[test]
fn replay_to_each_boundary_equals_the_recorded_prefix() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let full = ReplayRange::full(&movie).replay(&mut new_manager())?;
    assert_eq!(full.final_state_hash, movie.final_state_hash());
    assert_eq!(full.final_tick, movie.end_tick());
    for (index, step) in movie.steps().iter().enumerate() {
        let outcome =
            ReplayRange::new(&movie, None, Some(step.outcome_tick()))?.replay(&mut new_manager())?;
        // The replay to this boundary ends on the same recorded (tick, hash)
        // that the full replay verified as its prefix at that boundary.
        assert_eq!(outcome.range_from, movie.start_tick());
        assert_eq!(outcome.range_until, step.outcome_tick());
        assert_eq!(outcome.prefix_steps, 0);
        assert_eq!(outcome.window_steps, index + 1);
        assert_eq!(outcome.replayed_steps, index + 1);
        assert_eq!(outcome.final_tick, step.outcome_tick());
        assert_eq!(outcome.final_state_hash, step.state_hash());
        assert_eq!(outcome.total_steps, movie.step_count());
        assert_eq!(outcome.end_tick, movie.end_tick());
    }
    Ok(())
}

#[test]
fn windowed_replay_verifies_the_prefix_and_reports_counts() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let count = movie.step_count();
    let index = 5;
    let from = movie
        .steps()
        .get(index)
        .ok_or_else(|| "expected a recorded step".to_owned())?
        .outcome_tick();
    let outcome = ReplayRange::new(&movie, Some(from), None)?.replay(&mut new_manager())?;
    assert_eq!(outcome.range_from, from);
    assert_eq!(outcome.range_until, movie.end_tick());
    assert_eq!(outcome.prefix_steps, index + 1);
    assert_eq!(outcome.window_steps, count - 1 - index);
    assert_eq!(outcome.replayed_steps, count);
    assert_eq!(outcome.final_tick, movie.end_tick());
    assert_eq!(outcome.final_state_hash, movie.final_state_hash());

    // A divergence in the prefix is reported at its own index: a ranged replay
    // is never a passing window over an unverified prefix.
    let prefix_index = 2;
    let mut steps = movie.steps().to_vec();
    let recorded_hash = steps[prefix_index].state_hash();
    let altered_hash = flip_hash(recorded_hash);
    steps[prefix_index] = MovieStep::new(
        steps[prefix_index].outcome_tick(),
        steps[prefix_index].input(),
        altered_hash,
    );
    let divergent = movie_with_steps(&movie, steps)?;
    let error = match ReplayRange::new(&divergent, Some(from), None)?.replay(&mut new_manager()) {
        Ok(_) => {
            return Err("a divergent prefix must not replay cleanly"
                .to_owned()
                .into());
        }
        Err(error) => error,
    };
    match error {
        MovieError::Divergence {
            tick,
            input_index,
            expected,
            observed,
        } => {
            assert_eq!(input_index, prefix_index);
            assert_eq!(tick, movie.steps()[prefix_index].outcome_tick());
            assert_eq!(expected, altered_hash);
            assert_eq!(observed, recorded_hash);
        }
        other => {
            return Err(format!("expected a prefix divergence, got {other}").into());
        }
    }
    Ok(())
}

#[test]
fn seek_and_single_step_advance_are_new_invocations() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let steps = movie.steps();
    let count = steps.len();

    // Seek to an earlier boundary [start_tick, t_j].
    let earlier = 4;
    let outcome = ReplayRange::new(&movie, None, Some(steps[earlier].outcome_tick()))?
        .replay(&mut new_manager())?;
    assert_eq!(outcome.final_tick, steps[earlier].outcome_tick());
    assert_eq!(outcome.final_state_hash, steps[earlier].state_hash());

    // until = t_0 is the single-step window with an empty prefix.
    let first =
        ReplayRange::new(&movie, None, Some(steps[0].outcome_tick()))?.replay(&mut new_manager())?;
    assert_eq!(first.prefix_steps, 0);
    assert_eq!(first.window_steps, 1);
    assert_eq!(first.replayed_steps, 1);
    assert_eq!(first.final_tick, steps[0].outcome_tick());
    assert_eq!(first.final_state_hash, steps[0].state_hash());

    // Forward advance by exactly one recorded step is a new invocation whose
    // window contains that one step.
    for (index, pair) in steps.windows(2).enumerate() {
        let advance = ReplayRange::new(
            &movie,
            Some(pair[0].outcome_tick()),
            Some(pair[1].outcome_tick()),
        )?
        .replay(&mut new_manager())?;
        assert_eq!(advance.prefix_steps, index + 1);
        assert_eq!(advance.window_steps, 1);
        assert_eq!(advance.replayed_steps, index + 2);
        assert_eq!(advance.final_tick, pair[1].outcome_tick());
        assert_eq!(advance.final_state_hash, pair[1].state_hash());
    }

    // from = t_{n-2}, until = end_tick is exactly the last recorded step.
    let last_boundary = ReplayRange::new(&movie, Some(steps[count - 2].outcome_tick()), None)?
        .replay(&mut new_manager())?;
    assert_eq!(last_boundary.prefix_steps, count - 1);
    assert_eq!(last_boundary.window_steps, 1);
    assert_eq!(last_boundary.final_tick, movie.end_tick());
    assert_eq!(last_boundary.final_state_hash, movie.final_state_hash());

    // from = start_tick is the empty prefix.
    let from_start = ReplayRange::new(
        &movie,
        Some(movie.start_tick()),
        Some(steps[3].outcome_tick()),
    )?
    .replay(&mut new_manager())?;
    assert_eq!(from_start.prefix_steps, 0);
    assert_eq!(from_start.window_steps, 4);
    Ok(())
}

#[test]
fn rewind_and_repeat_carry_no_state() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let bytes_before = movie.to_bytes()?;
    let steps = movie.steps().to_vec();
    let count = steps.len();
    let late = ReplayRange::new(&movie, Some(steps[count - 2].outcome_tick()), None)?;
    let early = ReplayRange::new(
        &movie,
        Some(steps[1].outcome_tick()),
        Some(steps[4].outcome_tick()),
    )?;

    let mut manager = new_manager();
    let bystander = manager.create_session()?;
    step_many(&mut manager, bystander, 9, 2)?;
    let bystander_before = machine(&manager, bystander)?;

    let late_first = late.replay(&mut manager)?;
    assert_eq!(
        manager.len(),
        2,
        "one bystander plus one imported replay session"
    );

    // Rewind: a new invocation with an earlier window in the same manager.
    let early_after_late = early.replay(&mut manager)?;
    let early_first = early.replay(&mut new_manager())?;
    assert_eq!(early_after_late, early_first);
    assert_eq!(manager.len(), 3);

    // Repeating the late replay after a rewind does not depend on history.
    let late_again = late.replay(&mut manager)?;
    assert_eq!(late_again, late_first);
    assert_eq!(manager.len(), 4);

    // The bystander and the movie bytes are untouched by every invocation.
    assert_eq!(machine(&manager, bystander)?, bystander_before);
    assert_eq!(movie.to_bytes()?, bytes_before);
    Ok(())
}

#[test]
fn zero_step_movie_accepts_only_its_full_range() -> TestResult {
    let movie = record_from(4, &[])?;
    assert_eq!(movie.step_count(), 0);
    let checkpoint = Snapshot::from_bytes(movie.snapshot_bytes())?;
    let full = ReplayRange::new(&movie, None, None)?.replay(&mut new_manager())?;
    assert_eq!(full.range_from, movie.start_tick());
    assert_eq!(full.range_until, movie.start_tick());
    assert_eq!(full.prefix_steps, 0);
    assert_eq!(full.window_steps, 0);
    assert_eq!(full.replayed_steps, 0);
    assert_eq!(full.final_tick, movie.start_tick());
    assert_eq!(full.final_state_hash, checkpoint.state_hash());
    assert_eq!(full.final_state_hash, movie.checkpoint_state_hash());

    let explicit = ReplayRange::new(&movie, Some(movie.start_tick()), Some(movie.start_tick()))?;
    assert_eq!(explicit.replay(&mut new_manager())?, full);

    // No boundary beyond the embedded checkpoint exists for a zero-step movie.
    let after = Tick::from_raw(movie.start_tick().raw() + 1);
    assert!(matches!(
        ReplayRange::new(&movie, Some(after), None),
        Err(MovieError::RangeInverted { .. })
    ));
    assert!(matches!(
        ReplayRange::new(&movie, None, Some(after)),
        Err(MovieError::RangeAfterEnd { .. })
    ));
    Ok(())
}

#[test]
fn invalid_ranges_are_typed_before_import() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let start = movie.start_tick();
    let end = movie.end_tick();
    let steps = movie.steps().to_vec();

    // Bounds, inversion, alignment and emptiness have their own typed variants.
    assert!(matches!(
        ReplayRange::new(&movie, Some(Tick::from_raw(start.raw() - 1)), None),
        Err(MovieError::RangeBeforeStart { from, start_tick })
            if from == Tick::from_raw(start.raw() - 1) && start_tick == start
    ));
    assert!(matches!(
        ReplayRange::new(&movie, None, Some(Tick::from_raw(end.raw() + 1))),
        Err(MovieError::RangeAfterEnd { until, end_tick })
            if until == Tick::from_raw(end.raw() + 1) && end_tick == end
    ));
    assert!(matches!(
        ReplayRange::new(
            &movie,
            Some(steps[5].outcome_tick()),
            Some(steps[2].outcome_tick())
        ),
        Err(MovieError::RangeInverted { from, until })
            if from == steps[5].outcome_tick() && until == steps[2].outcome_tick()
    ));
    assert!(matches!(
        ReplayRange::new(
            &movie,
            Some(steps[5].outcome_tick()),
            Some(steps[5].outcome_tick())
        ),
        Err(MovieError::RangeEmpty { .. })
    ));
    assert!(matches!(
        ReplayRange::new(&movie, Some(end), None),
        Err(MovieError::RangeEmpty { .. })
    ));
    // until == start_tick is aligned and therefore an empty window, not an
    // unaligned tick.
    assert!(matches!(
        ReplayRange::new(&movie, None, Some(start)),
        Err(MovieError::RangeEmpty { .. })
    ));
    // Arbitrary 64-bit boundaries stay typed and never wrap or panic.
    assert!(matches!(
        ReplayRange::new(&movie, Some(Tick::from_raw(u64::MAX)), None),
        Err(MovieError::RangeInverted { .. })
    ));
    assert!(matches!(
        ReplayRange::new(&movie, None, Some(Tick::from_raw(u64::MAX))),
        Err(MovieError::RangeAfterEnd { .. })
    ));
    assert!(matches!(
        ReplayRange::new(
            &movie,
            Some(Tick::from_raw(u64::MAX)),
            Some(Tick::from_raw(u64::MAX))
        ),
        Err(MovieError::RangeAfterEnd { .. })
    ));
    // The Display text names both boundaries.
    let inverted = match ReplayRange::new(
        &movie,
        Some(steps[5].outcome_tick()),
        Some(steps[2].outcome_tick()),
    ) {
        Err(error) => error,
        Ok(_) => return Err("an inverted range must be rejected".to_owned().into()),
    };
    assert!(
        inverted
            .to_string()
            .contains(&steps[5].outcome_tick().to_string())
    );
    assert!(
        inverted
            .to_string()
            .contains(&steps[2].outcome_tick().to_string())
    );

    // Ticks that are neither the start tick nor a recorded outcome tick are
    // rejected; a fabricated movie with gaps makes such ticks representable.
    let gapped = gapped_movie(&movie, [5, 9, 14])?;
    assert!(matches!(
        ReplayRange::new(&gapped, Some(Tick::from_raw(4)), None),
        Err(MovieError::RangeFromUnaligned { tick }) if tick == Tick::from_raw(4)
    ));
    assert!(matches!(
        ReplayRange::new(&gapped, None, Some(Tick::from_raw(6))),
        Err(MovieError::RangeUntilUnaligned { tick }) if tick == Tick::from_raw(6)
    ));

    // A rejected range never touches the caller's manager: no session is
    // registered and no backend step is executed.
    let mut manager = new_manager();
    let bystander = manager.create_session()?;
    step_many(&mut manager, bystander, 7, 3)?;
    let before = manager.summaries()?;
    assert!(ReplayRange::new(&movie, Some(Tick::from_raw(start.raw() - 1)), None).is_err());
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.summaries()?, before);

    // Range validation comes before checkpoint import: an import-incompatible
    // movie still reports the range error, while its valid range reports the
    // existing typed snapshot-import failure.
    let program = SyntheticProgram::new(vec![0x01, 0x06, 0x00, 0x00])?;
    let mut foreign_manager = SessionManager::new(Arc::new(SyntheticBackendFactory::new(program)));
    let foreign_source = foreign_manager.create_session()?;
    step_many(&mut foreign_manager, foreign_source, 3, 2)?;
    let foreign = Movie::record(
        &mut foreign_manager,
        foreign_source,
        meta(2, Some("foreign"))?,
        &recorded_inputs()[..4],
    )?;
    let foreign_gapped = gapped_movie(&foreign, [4, 7, 10])?;
    assert!(matches!(
        ReplayRange::new(&foreign_gapped, Some(Tick::from_raw(3)), None),
        Err(MovieError::RangeFromUnaligned { tick }) if tick == Tick::from_raw(3)
    ));
    let valid = ReplayRange::new(&foreign_gapped, None, None)?;
    let import_error = match valid.replay(&mut new_manager()) {
        Ok(_) => {
            return Err("an incompatible checkpoint must not replay"
                .to_owned()
                .into());
        }
        Err(error) => error,
    };
    assert!(matches!(
        import_error,
        MovieError::SnapshotImport(SessionError::IncompatibleSnapshot(_))
    ));
    Ok(())
}

#[test]
fn increment_one_v1_fixture_bytes_still_decode_verify_and_replay() -> TestResult {
    // The increment-1 baseline fixture was recorded by the CLI from a fresh
    // synthetic session with 64 steps of input mask 5, movie id 7 and name
    // "baseline"; its SHA-256 is the recorded M3-T4 artifact hash. Re-creating
    // it here proves the v1 bytes are unchanged by this increment.
    let mut manager = new_manager();
    let source = manager.create_session()?;
    let inputs = vec![InputFrame::new(5); 64];
    let movie = Movie::record(&mut manager, source, meta(7, Some("baseline"))?, &inputs)?;
    let bytes = movie.to_bytes()?;
    assert_eq!(bytes.len(), 3369);
    assert_eq!(
        hash_bytes(&bytes).to_hex(),
        "b6b23b82d468fa973a8672f3a63a2bb36eab0be730d6ced06b175809412f042e"
    );
    assert_eq!(MOVIE_FORMAT_VERSION, 1);
    assert_eq!(MOVIE_INPUT_SCHEMA_VERSION, 1);

    let decoded = Movie::from_bytes(&bytes)?;
    assert_eq!(decoded, movie);
    assert_eq!(decoded.to_bytes()?, bytes, "re-encoding is byte-identical");

    let report = decoded.verify(&mut new_manager())?;
    assert_eq!(report.start_tick, Tick::ZERO);
    assert_eq!(report.end_tick, Tick::from_raw(64));
    assert_eq!(report.steps, 64);
    assert_eq!(report.final_tick, Tick::from_raw(64));
    assert_eq!(
        report.final_state_hash.to_hex(),
        "6addcba70379cd018749eff2b8bc2ae6205d8a269e200b6ca65a50921434c016"
    );

    // Ranged replay of the same increment-1 bytes works over the shared path.
    let window = ReplayRange::new(&decoded, Some(Tick::from_raw(16)), Some(Tick::from_raw(32)))?
        .replay(&mut new_manager())?;
    assert_eq!(window.range_from, Tick::from_raw(16));
    assert_eq!(window.range_until, Tick::from_raw(32));
    assert_eq!(window.prefix_steps, 16);
    assert_eq!(window.window_steps, 16);
    assert_eq!(window.replayed_steps, 32);
    assert_eq!(window.final_tick, Tick::from_raw(32));
    assert_eq!(window.final_state_hash, decoded.steps()[31].state_hash());
    Ok(())
}

#[test]
fn compare_reports_identical_and_ignores_metadata() -> TestResult {
    // Two independent recordings from the same unchanged source have different
    // embedded snapshot bytes (fresh snapshot ids) but the same checkpoint
    // identity and recorded steps; metadata differences are not equality fields.
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 3, 3)?;
    let first = Movie::record(
        &mut manager,
        source,
        meta(1, Some("branch-a"))?,
        &recorded_inputs(),
    )?;
    let second = Movie::record(
        &mut manager,
        source,
        meta(2, Some("branch-b"))?,
        &recorded_inputs(),
    )?;
    assert_ne!(first.to_bytes()?, second.to_bytes()?);
    assert_ne!(first.snapshot_bytes(), second.snapshot_bytes());
    assert_eq!(first.start_tick(), second.start_tick());
    assert_eq!(
        first.checkpoint_state_hash(),
        second.checkpoint_state_hash()
    );
    assert_eq!(
        compare_movies(&first, &second).verdict,
        ComparisonVerdict::Identical
    );
    assert_eq!(
        compare_movies(&first, &first).verdict,
        ComparisonVerdict::Identical
    );

    // Input masks are not equality fields in this increment: equal recorded
    // (outcome tick, state hash) sequences compare identical.
    let mut steps = first.steps().to_vec();
    steps[0] = MovieStep::new(
        steps[0].outcome_tick(),
        InputFrame::new(steps[0].input().buttons() ^ 0xff),
        steps[0].state_hash(),
    );
    let input_difference = movie_with_steps(&first, steps)?;
    assert_ne!(first.to_bytes()?, input_difference.to_bytes()?);
    assert_eq!(
        compare_movies(&first, &input_difference).verdict,
        ComparisonVerdict::Identical
    );
    Ok(())
}

#[test]
fn compare_reports_the_exact_first_divergence() -> TestResult {
    let movie = record_from(3, &recorded_inputs())?;
    let index = 2;
    let later = 7;
    let mut steps = movie.steps().to_vec();
    let original_hash = steps[index].state_hash();
    let altered_hash = flip_hash(original_hash);
    steps[index] = MovieStep::new(
        steps[index].outcome_tick(),
        steps[index].input(),
        altered_hash,
    );
    let later_original = steps[later].state_hash();
    steps[later] = MovieStep::new(
        steps[later].outcome_tick(),
        steps[later].input(),
        flip_hash(later_original),
    );
    let divergent = movie_with_steps(&movie, steps)?;

    let report = compare_movies(&movie, &divergent);
    match report.verdict {
        ComparisonVerdict::StepDivergence {
            index: found,
            tick_a,
            tick_b,
            state_hash_a,
            state_hash_b,
        } => {
            assert_eq!(found, index);
            assert_eq!(tick_a, movie.steps()[index].outcome_tick());
            assert_eq!(tick_b, tick_a);
            assert_eq!(state_hash_a, original_hash);
            assert_eq!(state_hash_b, altered_hash);
        }
        other => {
            return Err(format!("expected the first divergence, got {other:?}").into());
        }
    }
    assert_eq!(report.steps_a, movie.step_count());
    assert_eq!(report.steps_b, movie.step_count());

    // The reversed comparison names the same index and swaps the sides.
    match compare_movies(&divergent, &movie).verdict {
        ComparisonVerdict::StepDivergence {
            index: found,
            tick_a,
            tick_b,
            state_hash_a,
            state_hash_b,
        } => {
            assert_eq!(found, index);
            assert_eq!(tick_a, tick_b);
            assert_eq!(tick_b, movie.steps()[index].outcome_tick());
            assert_eq!(state_hash_a, altered_hash);
            assert_eq!(state_hash_b, original_hash);
        }
        other => {
            return Err(format!("expected the reversed divergence, got {other:?}").into());
        }
    }

    // A tick-only alteration in a gapped movie reports differing ticks and
    // equal hashes at the same index.
    let base = gapped_movie(&movie, [5, 9, 14])?;
    let mut moved = base.steps().to_vec();
    moved[1] = MovieStep::new(Tick::from_raw(11), moved[1].input(), moved[1].state_hash());
    let moved_movie = movie_with_steps(&base, moved)?;
    match compare_movies(&base, &moved_movie).verdict {
        ComparisonVerdict::StepDivergence {
            index: found,
            tick_a,
            tick_b,
            state_hash_a,
            state_hash_b,
        } => {
            assert_eq!(found, 1);
            assert_eq!(tick_a, Tick::from_raw(9));
            assert_eq!(tick_b, Tick::from_raw(11));
            assert_eq!(state_hash_a, state_hash_b);
        }
        other => {
            return Err(format!("expected a tick-only divergence, got {other:?}").into());
        }
    }
    Ok(())
}

#[test]
fn compare_reports_length_mismatch_separately() -> TestResult {
    let movie = record_from(0, &recorded_inputs())?;
    let shorter = movie_with_steps(&movie, movie.steps()[..10].to_vec())?;
    let report = compare_movies(&movie, &shorter);
    assert_eq!(
        report.verdict,
        ComparisonVerdict::LengthMismatch { common_steps: 10 }
    );
    assert_eq!(report.steps_a, 12);
    assert_eq!(report.steps_b, 10);

    let mut longer_steps = movie.steps().to_vec();
    longer_steps.push(MovieStep::new(
        Tick::from_raw(movie.end_tick().raw() + 1),
        InputFrame::new(0),
        StateHash::from_bytes([9; 32]),
    ));
    let longer = movie_with_steps(&movie, longer_steps)?;
    let report = compare_movies(&shorter, &longer);
    assert_eq!(
        report.verdict,
        ComparisonVerdict::LengthMismatch { common_steps: 10 }
    );
    assert_eq!(report.steps_a, 10);
    assert_eq!(report.steps_b, 13);

    // A first divergence in the common prefix wins over the length difference,
    // and both step counts remain visible.
    let mut divergent_shorter = shorter.steps().to_vec();
    divergent_shorter[3] = MovieStep::new(
        divergent_shorter[3].outcome_tick(),
        divergent_shorter[3].input(),
        flip_hash(divergent_shorter[3].state_hash()),
    );
    let divergent_shorter = movie_with_steps(&movie, divergent_shorter)?;
    let report = compare_movies(&movie, &divergent_shorter);
    assert!(matches!(
        report.verdict,
        ComparisonVerdict::StepDivergence { index: 3, .. }
    ));
    assert_eq!(report.steps_a, 12);
    assert_eq!(report.steps_b, 10);
    Ok(())
}

#[test]
fn compare_reports_checkpoint_identity_separately() -> TestResult {
    // Different start ticks from different capture points.
    let early = record_from(0, &[])?;
    let late = record_from(4, &[])?;
    let report = compare_movies(&early, &late);
    assert_eq!(report.verdict, ComparisonVerdict::CheckpointMismatch);
    assert_eq!(report.start_tick_a, Tick::ZERO);
    assert_eq!(report.start_tick_b, Tick::from_raw(4));
    assert_ne!(
        report.checkpoint_state_hash_a,
        report.checkpoint_state_hash_b
    );

    // Same start tick, different emulated checkpoint state.
    let mut first_manager = new_manager();
    let first_source = first_manager.create_session()?;
    step_many(&mut first_manager, first_source, 1, 3)?;
    let first = Movie::record(
        &mut first_manager,
        first_source,
        meta(10, Some("first"))?,
        &[],
    )?;
    let mut second_manager = new_manager();
    let second_source = second_manager.create_session()?;
    step_many(&mut second_manager, second_source, 2, 3)?;
    let second = Movie::record(
        &mut second_manager,
        second_source,
        meta(11, Some("second"))?,
        &[],
    )?;
    assert_eq!(first.start_tick(), second.start_tick());
    assert_ne!(
        first.checkpoint_state_hash(),
        second.checkpoint_state_hash()
    );
    let report = compare_movies(&first, &second);
    assert_eq!(report.verdict, ComparisonVerdict::CheckpointMismatch);
    assert_eq!(report.steps_a, 0);
    assert_eq!(report.steps_b, 0);
    Ok(())
}

#[test]
fn compare_never_imports_a_checkpoint() -> TestResult {
    // Two movies recorded under a non-default synthetic configuration share a
    // checkpoint identity and recorded steps, so comparison returns identical.
    // This build rejects their checkpoint at replay import, which proves the
    // comparison entry point never uses the import path.
    let program = SyntheticProgram::new(vec![0x01, 0x06, 0x00, 0x00])?;
    let mut first_manager =
        SessionManager::new(Arc::new(SyntheticBackendFactory::new(program.clone())));
    let first_source = first_manager.create_session()?;
    step_many(&mut first_manager, first_source, 3, 2)?;
    let first = Movie::record(
        &mut first_manager,
        first_source,
        meta(1, Some("foreign-a"))?,
        &recorded_inputs()[..4],
    )?;
    let mut second_manager = SessionManager::new(Arc::new(SyntheticBackendFactory::new(program)));
    let second_source = second_manager.create_session()?;
    step_many(&mut second_manager, second_source, 3, 2)?;
    let second = Movie::record(
        &mut second_manager,
        second_source,
        meta(2, None)?,
        &recorded_inputs()[..4],
    )?;

    let report = compare_movies(&first, &second);
    assert_eq!(report.verdict, ComparisonVerdict::Identical);
    assert_eq!(report.start_tick_a, report.start_tick_b);
    assert_eq!(
        report.checkpoint_state_hash_a,
        report.checkpoint_state_hash_b
    );

    // Verification of the same movie still fails at the import boundary.
    let error = match first.verify(&mut new_manager()) {
        Ok(_) => {
            return Err("an incompatible checkpoint must not verify"
                .to_owned()
                .into());
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        MovieError::SnapshotImport(SessionError::IncompatibleSnapshot(_))
    ));
    Ok(())
}

#[test]
fn replay_outcome_is_deterministic_across_invocations() -> TestResult {
    let movie = record_from(2, &recorded_inputs())?;
    let range = ReplayRange::new(
        &movie,
        Some(movie.steps()[3].outcome_tick()),
        Some(movie.steps()[8].outcome_tick()),
    )?;
    let first: ReplayOutcome = range.replay(&mut new_manager())?;
    let second: ReplayOutcome = range.replay(&mut new_manager())?;
    assert_eq!(first, second);
    assert_eq!(first.replayed_steps, 9);
    assert_eq!(first.prefix_steps, 4);
    assert_eq!(first.window_steps, 5);
    assert_eq!(first.start_tick, movie.start_tick());
    assert_eq!(first.end_tick, movie.end_tick());
    assert_eq!(first.total_steps, movie.step_count());
    Ok(())
}

#[test]
fn ranged_replay_never_touches_the_recording_source() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 4, 5)?;
    let source_before = machine(&manager, source)?;
    let movie = Movie::record(
        &mut manager,
        source,
        meta(1, Some("source"))?,
        &recorded_inputs(),
    )?;
    assert_eq!(machine(&manager, source)?, source_before);
    let sessions_before = manager.len();

    // A ranged replay in the recording manager imports exactly one fresh
    // session and never advances or restores the source session.
    let outcome = ReplayRange::new(&movie, Some(movie.steps()[2].outcome_tick()), None)?
        .replay(&mut manager)?;
    assert_eq!(outcome.prefix_steps, 3);
    assert_eq!(outcome.final_tick, movie.end_tick());
    assert_eq!(machine(&manager, source)?, source_before);
    assert_eq!(manager.len(), sessions_before + 1);
    Ok(())
}
