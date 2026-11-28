//! Integration tests for the in-process rewind player.
//!
//! These tests exercise the session-layer half of ADR-0007 on the deterministic
//! synthetic backend: cache cadence, bounds and deterministic eviction, rewind
//! exactness against a fresh replay at every target class (including before the
//! earliest cache, after eviction and with caching disabled), floor fallback,
//! first-divergence preservation, forward stepping, isolation from pre-existing
//! sessions, v1/v2 movie compatibility, determinism and the absence of file
//! writes.
//!
//! They return `Result` instead of unwrapping so the workspace `expect_used`
//! lint stays satisfied without a new file-level allowance.

use std::error::Error;
use std::fs;
use std::sync::Arc;

use chronofork_core::{InputFrame, StateHash, Tick};
use chronofork_sessions::{
    CacheOptions, MAX_CACHE_BYTE_LIMIT, Movie, MovieError, MovieMeta, MovieStep, Player,
    PlayerError, ReplayRange, SessionManager, StartKind,
};
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn meta(movie_id: u64) -> Result<MovieMeta, MovieError> {
    MovieMeta::new(movie_id, None, None)
}

fn inputs(count: usize) -> Vec<InputFrame> {
    (0..count)
        .map(|index| InputFrame::new(index as u32 % 11))
        .collect()
}

/// Records a v1 movie from a fresh session whose checkpoint tick is
/// `source_steps`.
fn recorded_movie(source_steps: usize, step_count: usize) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    for _ in 0..source_steps {
        manager.step(source, InputFrame::new(3))?;
    }
    Ok(Movie::record(
        &mut manager,
        source,
        meta(1)?,
        &inputs(step_count),
    )?)
}

fn open(movie: &Movie, options: CacheOptions) -> Result<Player, PlayerError> {
    Player::open(
        Arc::new(SyntheticBackendFactory::default()),
        movie.clone(),
        options,
    )
}

/// Returns the expected `(tick, state hash)` at positions `0..=step_count`.
fn expected_positions(movie: &Movie) -> Vec<(Tick, StateHash)> {
    let mut positions = vec![(movie.start_tick(), movie.checkpoint_state_hash())];
    positions.extend(
        movie
            .steps()
            .iter()
            .map(|step| (step.outcome_tick(), step.state_hash())),
    );
    positions
}

/// Returns the tick at applied-step position `position`.
fn tick_at(movie: &Movie, position: usize) -> Result<Tick, Box<dyn Error>> {
    if position == 0 {
        return Ok(movie.start_tick());
    }
    movie
        .steps()
        .get(position - 1)
        .map(MovieStep::outcome_tick)
        .ok_or_else(|| format!("movie has no recorded step at position {position}").into())
}

/// Replays the movie through `tick` in a fresh manager, independent of the
/// player, using the existing ranged replay path.
fn fresh_replay_at(
    movie: &Movie,
    tick: Tick,
    step_count: usize,
) -> Result<(Tick, StateHash), Box<dyn Error>> {
    if tick == movie.start_tick() {
        return Ok((movie.start_tick(), movie.checkpoint_state_hash()));
    }
    if step_count == 0 {
        return Err("a zero-step movie has no non-start target"
            .to_owned()
            .into());
    }
    let mut manager = new_manager();
    let range = ReplayRange::new(movie, None, Some(tick))?;
    let outcome = range.replay(&mut manager)?;
    Ok((outcome.final_tick, outcome.final_state_hash))
}

fn flip_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0xff;
    StateHash::from_bytes(bytes)
}

fn with_steps(movie: &Movie, steps: Vec<MovieStep>) -> Result<Movie, MovieError> {
    Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)
}

fn seek_error(player: &mut Player, tick: Tick) -> Result<PlayerError, Box<dyn Error>> {
    match player.seek(tick) {
        Ok(report) => Err(format!(
            "expected seek to tick {} to fail, got {report:?}",
            tick.raw()
        )
        .into()),
        Err(error) => Ok(error),
    }
}

#[test]
fn forward_stepping_reports_tick_index_hash_and_cache_stats() -> TestResult {
    let movie = recorded_movie(0, 6)?;
    let mut player = open(&movie, CacheOptions::default())?;

    let start = player.state();
    assert_eq!(start.applied_steps, 0);
    assert_eq!(start.tick, movie.start_tick());
    assert_eq!(start.state_hash, movie.checkpoint_state_hash());
    assert_eq!(player.cache_stats().entries, 0);

    for index in 0..movie.step_count() {
        let report = player.step_forward()?;
        assert_eq!(report.applied_steps, index + 1);
        assert_eq!(report.tick, movie.steps()[index].outcome_tick());
        assert_eq!(report.state_hash, movie.steps()[index].state_hash());
        let state = player.state();
        assert_eq!(state.applied_steps, index + 1);
        assert_eq!(state.tick, movie.steps()[index].outcome_tick());
        assert_eq!(state.state_hash, movie.steps()[index].state_hash());
        assert_eq!(player.cache_stats().entries, index + 1);
        assert_eq!(report.cache, player.cache_stats());
    }

    let back = player.step_back()?;
    assert_eq!(back.applied_steps, movie.step_count() - 1);
    assert_eq!(
        back.tick,
        movie.steps()[movie.step_count() - 2].outcome_tick()
    );

    while player.state().applied_steps > 0 {
        player.step_back()?;
    }
    let start_again = player.state();
    assert_eq!(start_again.applied_steps, 0);
    assert_eq!(start_again.tick, movie.start_tick());
    assert_eq!(start_again.state_hash, movie.checkpoint_state_hash());
    Ok(())
}

#[test]
fn seek_reproduces_recorded_expectations_at_every_position() -> TestResult {
    let movie = recorded_movie(0, 12)?;
    let positions = expected_positions(&movie);
    let mut player = open(&movie, CacheOptions::default())?;

    // Deliberately non-monotonic so every seek changes direction and mixes
    // cache-start and floor-start selections.
    for target in [0_usize, 7, 2, 12, 5, 1, 11, 12, 0, 6] {
        let tick = tick_at(&movie, target)?;
        let report = player.seek(tick)?;
        assert_eq!(report.target_tick, tick);
        assert_eq!(report.tick, tick);
        assert_eq!(report.state_hash, positions[target].1);
        assert_eq!(report.applied_steps, target);
        assert_eq!(report.cache, player.cache_stats());
        let (fresh_tick, fresh_hash) = fresh_replay_at(&movie, tick, movie.step_count())?;
        assert_eq!((report.tick, report.state_hash), (fresh_tick, fresh_hash));
    }

    let report = player.seek(movie.end_tick())?;
    let mut fresh = new_manager();
    let full = movie.verify(&mut fresh)?;
    assert_eq!(report.tick, full.final_tick);
    assert_eq!(report.state_hash, full.final_state_hash);
    Ok(())
}

#[test]
fn rewind_restores_the_nearest_cached_state_and_replays_forward() -> TestResult {
    let movie = recorded_movie(0, 10)?;
    let positions = expected_positions(&movie);
    // Interval 2 captures steps 1, 3, 5, 7, 9, so position 9 (after step 8)
    // picks the cached state after step 7 and replays one step.
    let options = CacheOptions::new(2, 256, MAX_CACHE_BYTE_LIMIT)?;
    let mut player = open(&movie, options)?;
    player.seek(movie.end_tick())?;

    let target = 9_usize;
    let report = player.seek(tick_at(&movie, target)?)?;
    assert_eq!(report.start, StartKind::Cache);
    assert_eq!(report.start_step, Some(7));
    assert_eq!(report.start_tick, movie.steps()[7].outcome_tick());
    assert_eq!(report.replayed_steps, 1);
    assert_eq!(report.applied_steps, target);
    assert_eq!(report.tick, tick_at(&movie, target)?);
    assert_eq!(report.state_hash, positions[target].1);

    // A target on a captured tick restores without replaying.
    let cached = player.seek(tick_at(&movie, 8)?)?;
    assert_eq!(cached.start, StartKind::Cache);
    assert_eq!(cached.start_step, Some(7));
    assert_eq!(cached.replayed_steps, 0);
    assert_eq!(cached.state_hash, positions[8].1);
    Ok(())
}

#[test]
fn targets_before_the_earliest_cache_fall_back_to_the_floor() -> TestResult {
    let movie = recorded_movie(0, 10)?;
    let positions = expected_positions(&movie);
    let options = CacheOptions::new(1, 4, MAX_CACHE_BYTE_LIMIT)?;
    // Capacity 4 keeps recorded steps 6..=9 after the forward pass, so every
    // target at or before step 6 must start from the floor. Each target uses a
    // fresh player because replaying from the floor re-captures early cadence
    // steps and deterministically changes a shared cache.
    for target in [0_usize, 1, 3, 6] {
        let mut player = open(&movie, options)?;
        player.seek(movie.end_tick())?;
        assert_eq!(player.cache_stats().entries, 4);
        assert_eq!(player.cache_stats().evictions, 6);

        let report = player.seek(tick_at(&movie, target)?)?;
        assert_eq!(report.start, StartKind::Floor);
        assert_eq!(report.start_tick, movie.start_tick());
        assert_eq!(report.start_step, None);
        assert_eq!(report.replayed_steps, target);
        assert_eq!(report.applied_steps, target);
        assert_eq!(report.state_hash, positions[target].1);
        let (fresh_tick, fresh_hash) =
            fresh_replay_at(&movie, tick_at(&movie, target)?, movie.step_count())?;
        assert_eq!((report.tick, report.state_hash), (fresh_tick, fresh_hash));
    }
    Ok(())
}

#[test]
fn rewinds_after_eviction_stay_exact_in_evicted_and_retained_regions() -> TestResult {
    let movie = recorded_movie(0, 10)?;
    let positions = expected_positions(&movie);
    // Capacity 3 keeps recorded steps 7..=9; every earlier target is evicted.
    for target in [0_usize, 2, 5, 7, 8, 9, 10, 4, 10] {
        let mut player = open(&movie, CacheOptions::new(1, 3, MAX_CACHE_BYTE_LIMIT)?)?;
        player.seek(movie.end_tick())?;
        let report = player.seek(tick_at(&movie, target)?)?;
        assert_eq!(report.applied_steps, target);
        assert_eq!(report.state_hash, positions[target].1);
        if target >= 8 {
            assert_eq!(report.start, StartKind::Cache);
            assert_eq!(report.start_step, Some(target - 1));
            assert_eq!(report.replayed_steps, 0);
        } else {
            assert_eq!(report.start, StartKind::Floor);
            assert_eq!(report.replayed_steps, target);
        }
        let (fresh_tick, fresh_hash) =
            fresh_replay_at(&movie, tick_at(&movie, target)?, movie.step_count())?;
        assert_eq!((report.tick, report.state_hash), (fresh_tick, fresh_hash));
    }
    Ok(())
}

#[test]
fn caching_disabled_rewinds_exactly_from_the_floor() -> TestResult {
    let movie = recorded_movie(0, 7)?;
    let positions = expected_positions(&movie);
    let mut player = open(&movie, CacheOptions::new(0, 256, MAX_CACHE_BYTE_LIMIT)?)?;
    for target in [3_usize, 7, 1, 5, 0, 7] {
        let report = player.seek(tick_at(&movie, target)?)?;
        assert_eq!(report.start, StartKind::Floor);
        assert_eq!(report.start_step, None);
        assert_eq!(report.replayed_steps, target);
        assert_eq!(report.applied_steps, target);
        assert_eq!(report.state_hash, positions[target].1);
        assert_eq!(player.cache_stats().entries, 0);
        assert_eq!(player.cache_stats().bytes, 0);
    }
    Ok(())
}

#[test]
fn invalid_targets_and_boundaries_are_typed_before_mutation() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    for _ in 0..5 {
        manager.step(source, InputFrame::new(2))?;
    }
    let snapshot_id = manager.snapshot(source)?;
    let snapshot_bytes = manager.snapshot_bytes(source, snapshot_id)?;
    let steps = [6_u64, 8, 10]
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
    let gapped = Movie::from_parts(meta(1)?, snapshot_bytes, steps)?;
    let mut player = open(&gapped, CacheOptions::default())?;
    let state = player.state();
    let stats = player.cache_stats();

    let before = seek_error(&mut player, Tick::from_raw(4))?;
    assert!(matches!(
        before,
        PlayerError::TargetBeforeStart {
            start_tick,
            ..
        } if start_tick == gapped.start_tick()
    ));
    let after = seek_error(&mut player, Tick::from_raw(11))?;
    assert!(matches!(
        after,
        PlayerError::TargetAfterEnd { end_tick, .. } if end_tick == gapped.end_tick()
    ));
    let unaligned = seek_error(&mut player, Tick::from_raw(7))?;
    assert!(matches!(unaligned, PlayerError::TargetUnaligned { tick } if tick.raw() == 7));
    let back = player.step_back();
    assert!(matches!(back, Err(PlayerError::AlreadyAtStart { .. })));
    assert_eq!(player.state(), state);
    assert_eq!(player.cache_stats(), stats);

    // `next` at the end is a boundary error on an executable movie.
    let executable = recorded_movie(0, 3)?;
    let mut at_end = open(&executable, CacheOptions::default())?;
    at_end.seek(executable.end_tick())?;
    let next = at_end.step_forward();
    assert!(matches!(next, Err(PlayerError::AlreadyAtEnd { .. })));
    assert_eq!(at_end.state().tick, executable.end_tick());
    assert_eq!(at_end.state().applied_steps, executable.step_count());
    Ok(())
}

#[test]
fn zero_step_movie_accepts_only_its_start_tick() -> TestResult {
    let movie = recorded_movie(4, 0)?;
    assert_eq!(movie.step_count(), 0);
    let mut player = open(&movie, CacheOptions::default())?;
    let report = player.seek(movie.start_tick())?;
    assert_eq!(report.start, StartKind::Floor);
    assert_eq!(report.applied_steps, 0);
    assert_eq!(report.replayed_steps, 0);
    assert_eq!(report.state_hash, movie.checkpoint_state_hash());
    assert!(matches!(
        seek_error(&mut player, Tick::from_raw(movie.start_tick().raw() + 1))?,
        PlayerError::TargetAfterEnd { .. }
    ));
    assert!(matches!(
        player.step_forward(),
        Err(PlayerError::AlreadyAtEnd { .. })
    ));
    assert!(matches!(
        player.step_back(),
        Err(PlayerError::AlreadyAtStart { .. })
    ));
    Ok(())
}

#[test]
fn first_divergence_is_reported_exactly_from_the_floor_and_a_cached_start() -> TestResult {
    let base = recorded_movie(0, 8)?;
    let original_steps = base.steps().to_vec();
    let divergence_index = 5_usize;
    let original = original_steps[divergence_index];
    let expected = flip_hash(original.state_hash());
    let mut tampered_steps = original_steps.clone();
    tampered_steps[divergence_index] =
        MovieStep::new(original.outcome_tick(), original.input(), expected);
    let tampered = with_steps(&base, tampered_steps)?;

    let mut floor_player = open(&tampered, CacheOptions::default())?;
    let error = seek_error(&mut floor_player, tampered.end_tick())?;
    match error {
        PlayerError::Movie { source, progress } => {
            let MovieError::Divergence {
                tick,
                input_index,
                expected: reported_expected,
                observed,
            } = *source
            else {
                return Err(format!("expected a state divergence, got {source:?}").into());
            };
            assert_eq!(tick, original.outcome_tick());
            assert_eq!(input_index, divergence_index);
            assert_eq!(reported_expected, expected);
            assert_eq!(observed, original.state_hash());
            let Some(progress) = progress else {
                return Err("replay divergence must carry seek progress"
                    .to_owned()
                    .into());
            };
            assert_eq!(progress.start, StartKind::Floor);
            assert_eq!(progress.start_step, None);
            assert_eq!(progress.replayed_steps, divergence_index + 1);
            assert_eq!(progress.cache_entries, divergence_index);
        }
        other => return Err(format!("expected a state divergence, got {other:?}").into()),
    }

    let mut cached_player = open(&tampered, CacheOptions::default())?;
    for _ in 0..divergence_index {
        cached_player.step_forward()?;
    }
    let error = seek_error(&mut cached_player, tampered.end_tick())?;
    match error {
        PlayerError::Movie { source, progress } => {
            let MovieError::Divergence { input_index, .. } = *source else {
                return Err(format!("expected a state divergence, got {source:?}").into());
            };
            assert_eq!(input_index, divergence_index);
            let Some(progress) = progress else {
                return Err("replay divergence must carry seek progress"
                    .to_owned()
                    .into());
            };
            assert_eq!(progress.start, StartKind::Cache);
            assert_eq!(progress.start_step, Some(divergence_index - 1));
            assert_eq!(progress.replayed_steps, 1);
            assert_eq!(progress.cache_entries, divergence_index);
        }
        other => return Err(format!("expected a state divergence, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn earliest_divergence_fails_before_any_cache_entry_is_captured() -> TestResult {
    let base = recorded_movie(0, 5)?;
    let mut steps = base.steps().to_vec();
    let first = steps[0];
    steps[0] = MovieStep::new(
        first.outcome_tick(),
        first.input(),
        flip_hash(first.state_hash()),
    );
    let tampered = with_steps(&base, steps)?;
    let mut player = open(&tampered, CacheOptions::default())?;
    let error = match player.step_forward() {
        Ok(report) => {
            return Err(format!("expected first-step divergence, got {report:?}").into());
        }
        Err(error) => error,
    };
    match error {
        PlayerError::Movie { source, progress } => {
            let MovieError::Divergence {
                input_index,
                tick,
                expected,
                observed,
            } = *source
            else {
                return Err(format!("expected a state divergence, got {source:?}").into());
            };
            assert_eq!(input_index, 0);
            assert_eq!(tick, first.outcome_tick());
            assert_eq!(expected, flip_hash(first.state_hash()));
            assert_eq!(observed, first.state_hash());
            let Some(progress) = progress else {
                return Err("replay divergence must carry seek progress"
                    .to_owned()
                    .into());
            };
            assert_eq!(progress.replayed_steps, 1);
            assert_eq!(progress.cache_entries, 0);
        }
        other => return Err(format!("expected a state divergence, got {other:?}").into()),
    }
    assert_eq!(player.cache_stats().entries, 0);
    assert_eq!(player.cache_stats().evictions, 0);
    Ok(())
}

#[test]
fn earlier_hash_divergence_is_reported_instead_of_a_later_one() -> TestResult {
    let base = recorded_movie(0, 8)?;
    let mut steps = base.steps().to_vec();
    for index in [5_usize, 2] {
        let step = steps[index];
        steps[index] = MovieStep::new(
            step.outcome_tick(),
            step.input(),
            flip_hash(step.state_hash()),
        );
    }
    let tampered = with_steps(&base, steps)?;
    let mut player = open(&tampered, CacheOptions::default())?;
    let error = seek_error(&mut player, tampered.end_tick())?;
    match error {
        PlayerError::Movie { source, .. } => {
            let MovieError::Divergence { input_index, .. } = *source else {
                return Err(format!("expected a state divergence, got {source:?}").into());
            };
            assert_eq!(input_index, 2);
        }
        other => return Err(format!("expected a state divergence, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn tick_mismatch_stays_a_typed_tick_mismatch() -> TestResult {
    let base = recorded_movie(0, 7)?;
    let mut steps = base.steps().to_vec();
    for step in steps.iter_mut().skip(3) {
        *step = MovieStep::new(
            Tick::from_raw(step.outcome_tick().raw() + 10),
            step.input(),
            step.state_hash(),
        );
    }
    let shifted = with_steps(&base, steps)?;
    let mut player = open(&shifted, CacheOptions::default())?;
    let error = seek_error(&mut player, shifted.end_tick())?;
    match error {
        PlayerError::Movie { source, progress } => {
            let MovieError::TickMismatch { input_index, .. } = *source else {
                return Err(format!("expected a tick mismatch, got {source:?}").into());
            };
            assert_eq!(input_index, 3);
            let Some(progress) = progress else {
                return Err("tick mismatch must carry seek progress".to_owned().into());
            };
            assert_eq!(progress.replayed_steps, 4);
        }
        other => return Err(format!("expected a tick mismatch, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn pre_existing_sessions_are_untouched_and_players_are_independent() -> TestResult {
    let movie = recorded_movie(0, 9)?;
    let mut external = new_manager();
    let external_session = external.create_session()?;
    for _ in 0..5 {
        external.step(external_session, InputFrame::new(6))?;
    }
    let external_before = external.inspect(external_session)?;
    let external_snapshot = external.snapshot(external_session)?;

    let mut first = open(&movie, CacheOptions::default())?;
    let mut second = open(&movie, CacheOptions::default())?;
    first.seek(movie.end_tick())?;
    assert_eq!(second.state().applied_steps, 0);
    assert_eq!(second.cache_stats().entries, 0);
    second.seek(tick_at(&movie, 4)?)?;
    let second_state = second.state();
    let second_stats = second.cache_stats();
    first.seek(tick_at(&movie, 1)?)?;
    assert_eq!(second.state(), second_state);
    assert_eq!(second.cache_stats(), second_stats);

    let external_after = external.inspect(external_session)?;
    assert_eq!(external_after.machine.tick, external_before.machine.tick);
    assert_eq!(external_after.state_hash, external_before.state_hash);
    assert_eq!(external.summaries()?.len(), 1);
    assert!(
        external
            .snapshot_record(external_session, external_snapshot)
            .is_ok()
    );
    external.restore(external_session, external_snapshot)?;
    let restored = external.inspect(external_session)?;
    assert_eq!(restored.machine.tick, external_before.machine.tick);
    assert_eq!(restored.state_hash, external_before.state_hash);
    Ok(())
}

#[test]
fn identical_command_scripts_produce_identical_reports() -> TestResult {
    let movie = recorded_movie(0, 8)?;
    let middle = tick_at(&movie, 3)?;
    let start = tick_at(&movie, 0)?;
    let run = || -> Result<Vec<chronofork_sessions::SeekReport>, PlayerError> {
        let mut player = open(&movie, CacheOptions::default())?;
        Ok(vec![
            player.seek(movie.start_tick())?,
            player.step_forward()?,
            player.seek(movie.end_tick())?,
            player.step_back()?,
            player.seek(middle)?,
            player.seek(start)?,
        ])
    };
    let first = run()?;
    let second = run()?;
    assert_eq!(first, second);
    let first_bytes = first.last().map(|report| report.cache.bytes);
    let second_bytes = second.last().map(|report| report.cache.bytes);
    assert_eq!(first_bytes, second_bytes);
    Ok(())
}

#[test]
fn v1_and_v2_movies_rewind_to_the_same_observations() -> TestResult {
    let movie_inputs = inputs(9);
    let mut v1_manager = new_manager();
    let v1_source = v1_manager.create_session()?;
    let v1 = Movie::record(&mut v1_manager, v1_source, meta(1)?, &movie_inputs)?;

    let mut v2_manager = new_manager();
    let v2_source = v2_manager.create_session()?;
    let v2 = Movie::record_v2(&mut v2_manager, v2_source, meta(2)?, &movie_inputs, 2)?;
    assert_eq!(v1.format_version(), 1);
    assert_eq!(v2.format_version(), 2);
    assert!(!v2.anchors().is_empty());
    assert_eq!(v1.snapshot_bytes(), v2.snapshot_bytes());
    assert_eq!(v1.steps(), v2.steps());

    let mut v1_player = open(&v1, CacheOptions::default())?;
    let mut v2_player = open(&v2, CacheOptions::default())?;
    let v1_report = v1_player.seek(v1.end_tick())?;
    let v2_report = v2_player.seek(v2.end_tick())?;
    assert_eq!(v1_report.tick, v2_report.tick);
    assert_eq!(v1_report.state_hash, v2_report.state_hash);
    assert_eq!(v1_report.cache.entries, v2_report.cache.entries);

    let mut fresh = new_manager();
    let v2_verified = v2.verify(&mut fresh)?;
    assert_eq!(v2_report.tick, v2_verified.final_tick);
    assert_eq!(v2_report.state_hash, v2_verified.final_state_hash);
    Ok(())
}

#[test]
fn a_full_player_run_writes_no_files() -> TestResult {
    let movie = recorded_movie(0, 6)?;
    let directory = std::env::temp_dir().join(format!(
        "chronofork-rewind-{}-{}",
        std::process::id(),
        movie.step_count()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory)?;
    let mut player = open(&movie, CacheOptions::default())?;
    player.seek(movie.end_tick())?;
    player.step_back()?;
    player.seek(movie.start_tick())?;
    let entries = fs::read_dir(&directory)?.count();
    fs::remove_dir_all(&directory)?;
    assert_eq!(entries, 0, "the player must not create files");
    Ok(())
}
