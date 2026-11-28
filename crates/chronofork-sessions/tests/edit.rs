//! Library-side rerecord, truncation and branch editing tests.
//!
//! These tests exercise the ADR-0008 `chronofork_sessions::edit` contract on the
//! deterministic synthetic backend: verified prefix copy, deterministic suffix
//! re-simulation, truncation without executing dropped steps, anchor policies,
//! v1/v2 preservation, multi-device full-frame replacement, branch
//! identity/isolation, typed bounds, determinism and source non-mutation. Rebuild
//! determinism is checked with repeated edits in one manager, and a test-only
//! bulk-payload backend executes the per-anchor, cumulative and output-size writer
//! rejections. The tests return `Result` instead of unwrapping so the workspace
//! `expect_used` lint stays satisfied without a new allowance.

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use chronofork_backend_api::{
    Backend, BackendDescriptor, BackendError, BackendFactory, BackendId, BackendSnapshot,
    Capabilities, MachineInspection, StepOutcome, validate_envelope,
};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick, hash_bytes};
use chronofork_sessions::{
    AnchorPolicy, EditError, EditMeta, EditOutcome, MAX_ANCHOR_BYTES, MAX_ANCHOR_SNAPSHOT_BYTES,
    MAX_ANCHORS, MAX_MOVIE_BYTES, Movie, MovieError, MovieMeta, MovieStep, ReplayRange,
    SessionError, SessionManager, Snapshot, compare_movies,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

type TestResult = Result<(), Box<dyn Error>>;

/// One recorded `(outcome tick, input, state hash)` expectation tuple.
type StepRecord = (Tick, InputFrame, StateHash);

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn meta(movie_id: u64, name: Option<&str>) -> Result<MovieMeta, Box<dyn Error>> {
    Ok(MovieMeta::new(movie_id, name.map(str::to_owned), None)?)
}

fn records(movie: &Movie) -> Vec<StepRecord> {
    movie
        .steps()
        .iter()
        .map(|step| (step.outcome_tick(), step.input(), step.state_hash()))
        .collect()
}

fn first_steps(movie: &Movie, count: usize) -> Result<Vec<StepRecord>, Box<dyn Error>> {
    Ok(records(movie)
        .get(..count)
        .ok_or("movie is shorter than the requested prefix")?
        .to_vec())
}

fn flip_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0xff;
    StateHash::from_bytes(bytes)
}

/// Records a v1 movie of port-0 masks from a fresh session; the checkpoint is the
/// fresh initial state at tick 0.
fn record_v1(inputs: &[u32]) -> Result<Movie, Box<dyn Error>> {
    let frames: Vec<InputFrame> = inputs.iter().copied().map(InputFrame::new).collect();
    record_v1_frames(&frames)
}

fn record_v1_frames(inputs: &[InputFrame]) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    Ok(Movie::record(
        &mut manager,
        source,
        meta(1, Some("source"))?,
        inputs,
    )?)
}

fn record_v2(frames: &[InputFrame], interval: usize) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    Ok(Movie::record_v2(
        &mut manager,
        source,
        meta(1, Some("source"))?,
        frames,
        interval,
    )?)
}

fn device_frames() -> Vec<InputFrame> {
    vec![
        InputFrame::with_devices([1, 0, 0, 0], [0, 0, 0, 0], 0),
        InputFrame::with_devices([0, 2, 0, 0], [1, 0, 0, 0], 1),
        InputFrame::new(0),
        InputFrame::with_devices([0, 0, 4, 0], [0, -2, 0, 0], 0),
        InputFrame::with_devices([0, 0, 0, 8], [0, 0, 0, 3], 1 << 63),
        InputFrame::with_devices([5, 6, 7, 8], [i16::MIN, 1, 2, i16::MAX], 0xffff),
    ]
}

fn checkpoint_bytes(manager: &mut SessionManager) -> Result<Vec<u8>, Box<dyn Error>> {
    let session = manager.create_session()?;
    let snapshot = manager.snapshot(session)?;
    Ok(manager.snapshot_bytes(session, snapshot)?)
}

/// Builds a structurally valid v1 movie with fabricated expectation records.
///
/// The embedded checkpoint is a real canonical checkpoint of the default
/// synthetic backend, but the step ticks and hashes are caller-invisible fixtures:
/// these movies are only used where validation rejects before execution, or where
/// the edit starts at step 0 and therefore verifies no prefix.
fn fabricated_v1(
    movie_id: u64,
    step_ticks: &[u64],
    checkpoint: Vec<u8>,
) -> Result<Movie, Box<dyn Error>> {
    let steps = step_ticks
        .iter()
        .map(|tick| {
            MovieStep::new(
                Tick::from_raw(*tick),
                InputFrame::new(1),
                StateHash::from_bytes([0x5a; 32]),
            )
        })
        .collect();
    Ok(Movie::from_parts(meta(movie_id, None)?, checkpoint, steps)?)
}

/// Builds a structurally valid, anchor-less v2 movie with fabricated records.
fn fabricated_v2(step_count: usize, checkpoint: Vec<u8>) -> Result<Movie, Box<dyn Error>> {
    let steps = (0..step_count)
        .map(|index| {
            MovieStep::new(
                Tick::from_raw(index as u64 + 1),
                InputFrame::new(1),
                StateHash::from_bytes([0x5a; 32]),
            )
        })
        .collect();
    Ok(Movie::from_v2_parts(
        meta(1, None)?,
        checkpoint,
        Vec::new(),
        steps,
    )?)
}

fn branch_meta(movie_id: u64, name: Option<&str>) -> Result<EditMeta, Box<dyn Error>> {
    let mut meta = EditMeta::new();
    if let Some(name) = name {
        meta = meta.with_name(name);
    }
    Ok(meta.with_movie_id(movie_id))
}

fn expect_error(result: Result<EditOutcome, EditError>) -> Result<EditError, Box<dyn Error>> {
    match result {
        Ok(_) => Err("expected a typed edit rejection, got a movie".into()),
        Err(error) => Ok(error),
    }
}

/// Records a differential reference over the same checkpoint and input sequence.
fn reference_v1(
    source: &Movie,
    inputs: &[InputFrame],
    movie_id: u64,
) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let session = manager.create_session_from_snapshot_bytes(source.snapshot_bytes())?;
    Ok(Movie::record(
        &mut manager,
        session,
        meta(movie_id, None)?,
        inputs,
    )?)
}

fn reference_v2(
    source: &Movie,
    inputs: &[InputFrame],
    movie_id: u64,
) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let session = manager.create_session_from_snapshot_bytes(source.snapshot_bytes())?;
    Ok(Movie::record_v2(
        &mut manager,
        session,
        meta(movie_id, None)?,
        inputs,
        0,
    )?)
}

fn verify(movie: &Movie) -> Result<(), Box<dyn Error>> {
    let mut manager = new_manager();
    movie.verify(&mut manager)?;
    Ok(())
}

fn edited_inputs(source: &Movie, replacements: &[(usize, InputFrame)]) -> Vec<InputFrame> {
    let mut inputs: Vec<InputFrame> = source.steps().iter().map(MovieStep::input).collect();
    for (index, input) in replacements {
        inputs[*index] = *input;
    }
    inputs
}

#[test]
fn single_edit_copies_verified_prefix_and_resimulates_the_suffix() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let source_value = source.clone();
    let source_bytes = source.to_bytes()?;
    let mut manager = new_manager();
    let request = chronofork_sessions::MovieEdit::new().with_input(2, InputFrame::new(0x22))?;

    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;

    // The source value and its canonical bytes are unchanged.
    assert_eq!(source, source_value);
    assert_eq!(source.to_bytes()?, source_bytes);
    assert_eq!(manager.len(), 1);

    let edited = outcome.movie();
    assert_eq!(edited.format_version(), 1);
    assert_eq!(edited.input_schema_version(), 1);
    assert_eq!(edited.step_count(), 5);
    assert_eq!(first_steps(edited, 2)?, first_steps(&source, 2)?);
    let edited_step = edited.steps().get(2).ok_or("edited step is missing")?;
    assert_eq!(edited_step.input(), InputFrame::new(0x22));
    let source_step = source.steps().get(2).ok_or("source step is missing")?;
    assert_ne!(edited_step.state_hash(), source_step.state_hash());
    // A later suffix step keeps its recorded input but not its old expectation.
    let later = edited.steps().get(3).ok_or("suffix step missing")?;
    let previous = source.steps().get(3).ok_or("source suffix step missing")?;
    assert_eq!(later.input(), previous.input());
    assert_ne!(later.state_hash(), previous.state_hash());

    // The output is byte-canonical and verifies independently.
    assert_eq!(Movie::from_bytes(&edited.to_bytes()?)?, *edited);
    verify(edited)?;
    assert_eq!(
        compare_movies(&source, edited).verdict,
        chronofork_sessions::ComparisonVerdict::StepDivergence {
            index: 2,
            tick_a: source_step.outcome_tick(),
            tick_b: edited_step.outcome_tick(),
            state_hash_a: source_step.state_hash(),
            state_hash_b: edited_step.state_hash(),
        }
    );

    let report = outcome.report();
    assert_eq!(report.source_movie_id, 1);
    assert_eq!(report.movie_id, 2);
    assert_eq!(report.parent_movie_id, 1);
    assert_eq!(report.format_version, 1);
    assert_eq!(report.input_schema_version, 1);
    assert_eq!(report.edits, 1);
    assert_eq!(report.truncate_after, None);
    assert_eq!(report.prefix_steps, 2);
    assert_eq!(report.resimulated_steps, 3);
    assert_eq!(report.anchor_policy, AnchorPolicy::KeepPrefix);
    assert_eq!(report.anchors_retained, 0);
    assert_eq!(report.anchors_dropped, 0);
    assert_eq!(report.anchors_rebuilt, 0);
    Ok(())
}

#[test]
fn single_edit_matches_a_differential_full_rerecord() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let replacement = InputFrame::new(0x44);
    let request = chronofork_sessions::MovieEdit::new().with_input(2, replacement)?;
    let mut manager = new_manager();
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let inputs = edited_inputs(&source, &[(2, replacement)]);
    let reference = reference_v1(&source, &inputs, 9)?;
    assert_eq!(records(outcome.movie()), records(&reference));
    Ok(())
}

#[test]
fn multi_edit_uses_the_earliest_affected_step_once() -> TestResult {
    // The synthetic program consumes input on step indices 2, 7, 12, ... so these
    // two edits both change the state trajectory.
    let source = record_v1(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])?;
    let first = InputFrame::new(0xaa);
    let second = InputFrame::new(0xbb);
    let request = chronofork_sessions::MovieEdit::new()
        .with_input(2, first)?
        .with_input(7, second)?;
    let mut manager = new_manager();
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let edited = outcome.movie();
    assert_eq!(outcome.report().edits, 2);
    assert_eq!(outcome.report().prefix_steps, 2);
    assert_eq!(outcome.report().resimulated_steps, 10);
    assert_eq!(first_steps(edited, 2)?, first_steps(&source, 2)?);
    assert_eq!(
        edited.steps().get(2).ok_or("step 2 missing")?.input(),
        first
    );
    assert_eq!(
        edited.steps().get(7).ok_or("step 7 missing")?.input(),
        second
    );
    // Steps between the edits are re-simulated exactly once, not copied: the
    // recorded input is unchanged but its old expectation is stale.
    let source_step = source.steps().get(4).ok_or("source step 4 missing")?;
    let between = edited.steps().get(4).ok_or("step 4 missing")?;
    assert_eq!(between.input(), source_step.input());
    assert_ne!(between.state_hash(), source_step.state_hash());
    verify(edited)?;
    let reference = reference_v1(
        &source,
        &edited_inputs(&source, &[(2, first), (7, second)]),
        9,
    )?;
    assert_eq!(records(edited), records(&reference));
    Ok(())
}

#[test]
fn same_input_edit_still_resimulates_and_compares_identical() -> TestResult {
    let source = record_v1(&[3, 5, 7])?;
    let same = source.steps().get(1).ok_or("step 1 missing")?.input();
    let request = chronofork_sessions::MovieEdit::new().with_input(1, same)?;
    let mut manager = new_manager();
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    assert_eq!(records(outcome.movie()), records(&source));
    assert_eq!(
        compare_movies(&source, outcome.movie()).verdict,
        chronofork_sessions::ComparisonVerdict::Identical
    );
    assert_ne!(outcome.movie().to_bytes()?, source.to_bytes()?);
    Ok(())
}

#[test]
fn edit_at_the_first_and_last_step_use_an_empty_or_single_step_prefix() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9])?;
    let mut manager = new_manager();
    let at_first = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(0x10))?;
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &at_first, &EditMeta::new())?;
    assert_eq!(outcome.report().prefix_steps, 0);
    assert_eq!(outcome.report().resimulated_steps, 4);
    assert_eq!(
        outcome.movie().steps().first().map(MovieStep::input),
        Some(InputFrame::new(0x10))
    );

    let at_last = chronofork_sessions::MovieEdit::new().with_input(3, InputFrame::new(0x20))?;
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &at_last, &EditMeta::new())?;
    assert_eq!(outcome.report().prefix_steps, 3);
    assert_eq!(outcome.report().resimulated_steps, 1);
    assert_eq!(first_steps(outcome.movie(), 3)?, first_steps(&source, 3)?);
    verify(outcome.movie())?;
    Ok(())
}

#[test]
fn truncation_drops_the_suffix_without_executing_it() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let mut manager = new_manager();
    let request = chronofork_sessions::MovieEdit::new().with_truncate_after(2);
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let truncated = outcome.movie();
    assert_eq!(truncated.step_count(), 3);
    assert_eq!(first_steps(truncated, 3)?, first_steps(&source, 3)?);
    let kept_tick = source
        .steps()
        .get(2)
        .ok_or("step 2 missing")?
        .outcome_tick();
    assert_eq!(truncated.end_tick(), kept_tick);
    assert_eq!(outcome.report().prefix_steps, 3);
    assert_eq!(outcome.report().resimulated_steps, 0);
    assert_eq!(outcome.report().truncate_after, Some(2));
    verify(truncated)?;

    // A source with deliberately wrong records in the dropped suffix still edits:
    // the retained prefix is valid and no dropped step is executed.
    let mut steps: Vec<MovieStep> = source.steps().to_vec();
    let tampered = *steps.get(4).ok_or("step 4 missing")?;
    *steps.get_mut(4).ok_or("step 4 missing")? = MovieStep::new(
        tampered.outcome_tick(),
        tampered.input(),
        flip_hash(tampered.state_hash()),
    );
    let bad_suffix = Movie::from_parts(
        source.meta().clone(),
        source.snapshot_bytes().to_vec(),
        steps,
    )?;
    let outcome =
        chronofork_sessions::edit_movie(&bad_suffix, &mut manager, &request, &EditMeta::new())?;
    assert_eq!(outcome.movie().step_count(), 3);
    verify(outcome.movie())?;
    Ok(())
}

#[test]
fn edit_with_truncation_rejects_edits_in_the_dropped_suffix() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let mut manager = new_manager();
    let request = chronofork_sessions::MovieEdit::new()
        .with_input(1, InputFrame::new(0x11))?
        .with_truncate_after(3);
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    assert_eq!(outcome.movie().step_count(), 4);
    assert_eq!(outcome.report().prefix_steps, 1);
    assert_eq!(outcome.report().resimulated_steps, 3);
    verify(outcome.movie())?;

    let rejected = chronofork_sessions::MovieEdit::new()
        .with_input(3, InputFrame::new(0x11))?
        .with_truncate_after(2);
    let error = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &rejected,
        &EditMeta::new(),
    ))?;
    assert!(matches!(
        error,
        EditError::EditBeyondTruncation {
            index: 3,
            truncate_after: 2
        }
    ));
    Ok(())
}

#[test]
fn non_reproducing_prefix_fails_at_its_first_wrong_index() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let mut manager = new_manager();

    // A wrong state digest in the prefix is the existing typed Divergence.
    let mut steps: Vec<MovieStep> = source.steps().to_vec();
    let original = *steps.get(1).ok_or("step 1 missing")?;
    *steps.get_mut(1).ok_or("step 1 missing")? = MovieStep::new(
        original.outcome_tick(),
        original.input(),
        flip_hash(original.state_hash()),
    );
    let bad_hash = Movie::from_parts(
        source.meta().clone(),
        source.snapshot_bytes().to_vec(),
        steps,
    )?;
    let request = chronofork_sessions::MovieEdit::new().with_input(3, InputFrame::new(0x99))?;
    let error = expect_error(chronofork_sessions::edit_movie(
        &bad_hash,
        &mut manager,
        &request,
        &EditMeta::new(),
    ))?;
    assert!(matches!(
        error,
        EditError::Movie(MovieError::Divergence { input_index: 1, .. })
    ));

    // A wrong outcome tick is the existing typed TickMismatch, also first only.
    let checkpoint = checkpoint_bytes(&mut manager)?;
    let ticks: Vec<MovieStep> = (1..=5)
        .map(|index| {
            MovieStep::new(
                Tick::from_raw(index * 2),
                InputFrame::new(3),
                StateHash::from_bytes([0x5a; 32]),
            )
        })
        .collect();
    let bad_tick = Movie::from_parts(meta(1, None)?, checkpoint, ticks)?;
    let error = expect_error(chronofork_sessions::edit_movie(
        &bad_tick,
        &mut manager,
        &request,
        &EditMeta::new(),
    ))?;
    assert!(matches!(
        error,
        EditError::Movie(MovieError::TickMismatch {
            input_index: 0,
            expected: _,
            observed: _
        })
    ));
    verify(&source)?;
    Ok(())
}

#[test]
fn request_bounds_are_typed_before_any_session_or_step() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9, 11])?;
    let mut manager = new_manager();

    let empty = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &chronofork_sessions::MovieEdit::new(),
        &EditMeta::new(),
    ))?;
    assert!(matches!(empty, EditError::NoEditsRequested));

    let out_of_range = chronofork_sessions::MovieEdit::new().with_input(5, InputFrame::new(1))?;
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &out_of_range,
            &EditMeta::new()
        ))?,
        EditError::EditIndexOutOfRange {
            index: 5,
            step_count: 5
        }
    ));

    let duplicate = chronofork_sessions::MovieEdit::new()
        .with_input(1, InputFrame::new(1))?
        .with_input(1, InputFrame::new(2));
    assert!(matches!(
        duplicate,
        Err(EditError::DuplicateEditIndex { index: 1 })
    ));

    let truncate_range = chronofork_sessions::MovieEdit::new().with_truncate_after(5);
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &truncate_range,
            &EditMeta::new()
        ))?,
        EditError::TruncateIndexOutOfRange {
            index: 5,
            step_count: 5
        }
    ));

    let truncate_noop = chronofork_sessions::MovieEdit::new().with_truncate_after(4);
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &truncate_noop,
            &EditMeta::new()
        ))?,
        EditError::TruncationNoOp {
            index: 4,
            step_count: 5
        }
    ));

    let edit = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(1))?;
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_movie_id(1)
        ))?,
        EditError::BranchIdEqualsSource { movie_id: 1 }
    ));
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_anchor_policy(AnchorPolicy::Drop)
        ))?,
        EditError::AnchorsUnsupportedForV1
    ));
    // Even the default policy spelled explicitly is rejected on a v1 source: v1
    // has no anchor table, so an explicit anchor flag is never a silent no-op.
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_anchor_policy(AnchorPolicy::KeepPrefix)
        ))?,
        EditError::AnchorsUnsupportedForV1
    ));
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_name("bad\nname")
        ))?,
        EditError::Movie(MovieError::InvalidName(_))
    ));
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_movie_id(0)
        ))?,
        EditError::Movie(MovieError::InvalidMovieId)
    ));
    // Every rejection above happened before the import path ran.
    assert!(manager.is_empty());
    Ok(())
}

#[test]
fn anchor_and_id_bounds_are_typed_before_execution() -> TestResult {
    let mut checkpoint_manager = new_manager();
    let checkpoint = checkpoint_bytes(&mut checkpoint_manager)?;
    let source = record_v2(&device_frames(), 2)?;
    let mut manager = new_manager();
    let edit = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(1))?;

    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 0 })
        ))?,
        EditError::InvalidAnchorInterval {
            interval: 0,
            steps: 6
        }
    ));
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &source,
            &mut manager,
            &edit,
            &EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 7 })
        ))?,
        EditError::InvalidAnchorInterval { interval: 7, .. }
    ));

    // The predicted rebuild count is bounded before any checkpoint is captured.
    let huge = fabricated_v2(70_000, checkpoint)?;
    let huge_edit = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(1))?;
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &huge,
            &mut manager,
            &huge_edit,
            &EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 1 })
        ))?,
        EditError::TooManyRebuiltAnchors {
            limit: MAX_ANCHORS,
            ..
        }
    ));

    // A default id above `u64::MAX` overflows instead of wrapping.
    let steps: Vec<MovieStep> = (1..=3)
        .map(|index| {
            MovieStep::new(
                Tick::from_raw(index),
                InputFrame::new(1),
                StateHash::from_bytes([0x5a; 32]),
            )
        })
        .collect();
    let max_id = Movie::from_parts(meta(u64::MAX, None)?, checkpoint_source()?, steps)?;
    assert!(matches!(
        expect_error(chronofork_sessions::edit_movie(
            &max_id,
            &mut manager,
            &edit,
            &EditMeta::new()
        ))?,
        EditError::BranchIdOverflow {
            source_movie_id: u64::MAX
        }
    ));
    // Every rejection above happened before the import path ran.
    assert!(manager.is_empty());
    Ok(())
}

/// Returns one canonical checkpoint of the default synthetic backend.
fn checkpoint_source() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut manager = new_manager();
    checkpoint_bytes(&mut manager)
}

#[test]
fn branch_metadata_identity_and_dangling_parent() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9])?;
    let mut manager = new_manager();
    let request = chronofork_sessions::MovieEdit::new().with_input(1, InputFrame::new(0x31))?;

    // Default id is source + 1, the parent is the source, and the name is not
    // inherited even though the source has one.
    let default =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    assert_eq!(default.movie().meta().movie_id(), 2);
    assert_eq!(default.movie().meta().parent_movie_id(), Some(1));
    assert_eq!(default.movie().meta().name(), None);
    assert_eq!(default.report().parent_movie_id, 1);

    let explicit = chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &branch_meta(42, Some("branch"))?,
    )?;
    assert_eq!(explicit.movie().meta().movie_id(), 42);
    assert_eq!(explicit.movie().meta().parent_movie_id(), Some(1));
    assert_eq!(explicit.movie().meta().name(), Some("branch"));

    // Editing a branch sets the immediate parent to that branch, not the root.
    let chained = chronofork_sessions::edit_movie(
        explicit.movie(),
        &mut manager,
        &request,
        &branch_meta(43, None)?,
    )?;
    assert_eq!(chained.movie().meta().parent_movie_id(), Some(42));

    // The parent edge is passive: the output decodes and verifies without any
    // source lookup.
    assert_eq!(
        Movie::from_bytes(&chained.movie().to_bytes()?)?,
        *chained.movie()
    );
    verify(chained.movie())?;

    // Metadata never decides comparison equality.
    let renamed = chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &branch_meta(7, Some("renamed"))?,
    )?;
    assert_eq!(
        compare_movies(explicit.movie(), renamed.movie()).verdict,
        chronofork_sessions::ComparisonVerdict::Identical
    );
    assert_ne!(explicit.movie().to_bytes()?, renamed.movie().to_bytes()?);
    Ok(())
}

#[test]
fn pre_existing_sessions_and_the_source_are_untouched() -> TestResult {
    let source = record_v1(&[3, 5, 7])?;
    let source_bytes = source.to_bytes()?;
    let mut manager = new_manager();
    let existing = manager.create_session()?;
    manager.step(existing, InputFrame::new(7))?;
    let before = manager.inspect(existing)?;
    let request = chronofork_sessions::MovieEdit::new().with_input(1, InputFrame::new(0x21))?;
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let after = manager.inspect(existing)?;
    let before_hash = manager.state_hash(existing)?;
    assert_eq!(before.machine.tick, after.machine.tick);
    assert_eq!(
        manager.state_hash(existing)?,
        before_hash,
        "editing another movie must not change this session"
    );
    assert_eq!(manager.len(), 2);
    assert_eq!(source.to_bytes()?, source_bytes);
    verify(outcome.movie())?;
    Ok(())
}

#[test]
fn v2_keep_prefix_retains_only_valid_prefix_anchors() -> TestResult {
    let source = record_v2(&device_frames(), 2)?;
    assert_eq!(source.anchors().len(), 3);
    let mut manager = new_manager();
    let replacement = InputFrame::with_devices([9, 0, 0, 0], [0; 4], 0);
    let request = chronofork_sessions::MovieEdit::new().with_input(3, replacement)?;
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let edited = outcome.movie();
    assert_eq!(edited.format_version(), 2);
    assert_eq!(edited.input_schema_version(), 2);
    assert_eq!(outcome.report().prefix_steps, 3);
    assert_eq!(outcome.report().anchors_retained, 1);
    assert_eq!(outcome.report().anchors_dropped, 2);
    let retained = edited.anchors().first().ok_or("retained anchor missing")?;
    let source_anchor = source.anchors().first().ok_or("source anchor missing")?;
    assert_eq!(retained.step_index(), 1);
    assert_eq!(retained.snapshot_bytes(), source_anchor.snapshot_bytes());
    assert_eq!(edited.anchors().len(), 1);
    // The embedded checkpoint is byte-identical to the source's.
    assert_eq!(edited.snapshot_bytes(), source.snapshot_bytes());
    verify(edited)?;

    // A ranged replay can seek from the retained anchor on the output.
    let seek_tick = edited
        .steps()
        .get(retained.step_index())
        .ok_or("anchored step missing")?
        .outcome_tick();
    let outcome = ReplayRange::new(edited, Some(seek_tick), None)?.replay(&mut manager)?;
    assert!(outcome.anchored);
    assert_eq!(outcome.anchor_step, Some(1));
    assert_eq!(outcome.final_tick, edited.end_tick());
    assert_eq!(outcome.final_state_hash, edited.final_state_hash());
    Ok(())
}

#[test]
fn v2_drop_and_rebuild_anchor_policies_are_deterministic() -> TestResult {
    let source = record_v2(&device_frames(), 2)?;
    let replacement = InputFrame::with_devices([9, 0, 0, 0], [0; 4], 0);
    let request = chronofork_sessions::MovieEdit::new().with_input(3, replacement)?;

    let keep =
        chronofork_sessions::edit_movie(&source, &mut new_manager(), &request, &EditMeta::new())?;
    let drop = chronofork_sessions::edit_movie(
        &source,
        &mut new_manager(),
        &request,
        &EditMeta::new().with_anchor_policy(AnchorPolicy::Drop),
    )?;
    assert!(drop.movie().anchors().is_empty());
    assert_eq!(drop.report().anchors_retained, 0);
    assert_eq!(drop.report().anchors_dropped, 3);
    assert_eq!(drop.report().anchors_rebuilt, 0);
    assert_eq!(records(drop.movie()), records(keep.movie()));

    let rebuild = AnchorPolicy::Rebuild { interval: 2 };
    let rebuilt = chronofork_sessions::edit_movie(
        &source,
        &mut new_manager(),
        &request,
        &EditMeta::new().with_anchor_policy(rebuild),
    )?;
    assert_eq!(rebuilt.report().anchors_retained, 0);
    assert_eq!(rebuilt.report().anchors_dropped, 3);
    assert_eq!(rebuilt.report().anchors_rebuilt, 3);
    assert_eq!(rebuilt.movie().anchors().len(), 3);
    for (anchor, step_index) in rebuilt.movie().anchors().iter().zip([1_usize, 3, 5]) {
        assert_eq!(anchor.step_index(), step_index);
        let step = rebuilt
            .movie()
            .steps()
            .get(step_index)
            .ok_or("rebuilt anchor step missing")?;
        assert_eq!(anchor.tick(), step.outcome_tick());
        assert_eq!(anchor.state_hash(), step.state_hash());
    }
    verify(rebuilt.movie())?;

    // Two separate managers with identical inputs produce byte-identical output.
    let again = chronofork_sessions::edit_movie(
        &source,
        &mut new_manager(),
        &request,
        &EditMeta::new().with_anchor_policy(rebuild),
    )?;
    assert_eq!(again.movie().to_bytes()?, rebuilt.movie().to_bytes()?);

    // An anchor-less v2 source stays anchor-less under keep-prefix.
    let anchorless = record_v2(&device_frames(), 0)?;
    let anchorless_out = chronofork_sessions::edit_movie(
        &anchorless,
        &mut new_manager(),
        &request,
        &EditMeta::new(),
    )?;
    assert!(anchorless_out.movie().anchors().is_empty());
    assert_eq!(anchorless_out.report().anchors_retained, 0);
    assert_eq!(anchorless_out.report().anchors_dropped, 0);
    Ok(())
}

#[test]
fn rebuild_anchors_do_not_depend_on_manager_history() -> TestResult {
    let source = record_v2(&device_frames(), 2)?;
    let replacement = InputFrame::with_devices([9, 0, 0, 0], [0; 4], 0);
    let request = chronofork_sessions::MovieEdit::new().with_input(3, replacement)?;
    let meta = EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 2 });
    let mut manager = new_manager();
    // An unrelated session shifts the imported session ids of both edits, and the
    // first edit shifts the second edit's manager state further.
    manager.create_session()?;
    let first = chronofork_sessions::edit_movie(&source, &mut manager, &request, &meta)?;
    let second = chronofork_sessions::edit_movie(&source, &mut manager, &request, &meta)?;
    assert_eq!(first.movie().to_bytes()?, second.movie().to_bytes()?);
    assert_eq!(first.report(), second.report());
    assert_eq!(first.movie().anchors().len(), 3);
    // Rebuilt anchors carry the documented canonical identity, so their bytes do
    // not embed the caller's session allocations.
    for anchor in first.movie().anchors() {
        let snapshot = Snapshot::from_bytes(anchor.snapshot_bytes())?;
        assert_eq!(snapshot.id(), SnapshotId::from_raw(1));
        assert_eq!(snapshot.origin_session(), SessionId::from_raw(1));
        assert_eq!(snapshot.tick(), anchor.tick());
        assert_eq!(snapshot.state_hash(), anchor.state_hash());
    }
    verify(first.movie())?;
    // A rebuilt canonical anchor still imports and seeks through the existing
    // checkpoint path, including the anchor after the edited step.
    let anchor = first
        .movie()
        .anchors()
        .get(1)
        .ok_or("rebuilt anchor at the edited step is missing")?;
    assert_eq!(anchor.step_index(), 3);
    let mut seek_manager = new_manager();
    let outcome =
        ReplayRange::new(first.movie(), Some(anchor.tick()), None)?.replay(&mut seek_manager)?;
    assert!(outcome.anchored);
    assert_eq!(outcome.anchor_step, Some(3));
    assert_eq!(outcome.final_tick, first.movie().end_tick());
    assert_eq!(outcome.final_state_hash, first.movie().final_state_hash());
    Ok(())
}

#[test]
fn full_frame_v2_edit_encodes_every_device_field() -> TestResult {
    let source = record_v2(&device_frames(), 0)?;
    let replacement = InputFrame::with_devices(
        [0xdead_beef, 1, 2, 3],
        [i16::MIN, -1, 1, i16::MAX],
        u64::MAX,
    );
    let request = chronofork_sessions::MovieEdit::new().with_input(2, replacement)?;
    let mut manager = new_manager();
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    let edited = outcome.movie();
    assert_eq!(edited.format_version(), 2);
    let step = edited.steps().get(2).ok_or("edited step missing")?;
    assert_eq!(step.input(), replacement);
    assert_eq!(step.input().ports(), replacement.ports());
    assert_eq!(step.input().axes(), replacement.axes());
    assert_eq!(step.input().keys(), replacement.keys());
    assert_eq!(Movie::from_bytes(&edited.to_bytes()?)?, *edited);
    verify(edited)?;
    let reference = reference_v2(&source, &edited_inputs(&source, &[(2, replacement)]), 9)?;
    assert_eq!(records(edited), records(&reference));
    Ok(())
}

#[test]
fn identical_requests_produce_identical_bytes_from_decoded_sources() -> TestResult {
    let source = record_v1(&[3, 5, 7, 9])?;
    let request = chronofork_sessions::MovieEdit::new()
        .with_input(1, InputFrame::new(0x12))?
        .with_input(3, InputFrame::new(0x34))?;
    let meta = branch_meta(21, Some("deterministic"))?;
    let first = chronofork_sessions::edit_movie(&source, &mut new_manager(), &request, &meta)?;
    let decoded = Movie::from_bytes(&source.to_bytes()?)?;
    let second = chronofork_sessions::edit_movie(&decoded, &mut new_manager(), &request, &meta)?;
    assert_eq!(first.movie().to_bytes()?, second.movie().to_bytes()?);
    assert_eq!(first.report(), second.report());
    Ok(())
}

fn halting_manager(program: Vec<u8>) -> Result<SessionManager, Box<dyn Error>> {
    let program = SyntheticProgram::new(program)?;
    Ok(SessionManager::new(Arc::new(SyntheticBackendFactory::new(
        program,
    ))))
}

#[test]
fn halt_before_the_last_output_step_is_a_typed_failure() -> TestResult {
    // Step 0 is a no-op, step 1 halts the machine.
    let mut manager = halting_manager(vec![0x00, 0xff])?;
    let checkpoint = checkpoint_bytes(&mut manager)?;
    let source = fabricated_v1(1, &[1, 2, 3], checkpoint)?;
    let request = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(1))?;
    let error = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &EditMeta::new(),
    ))?;
    assert!(matches!(
        error,
        EditError::Step {
            step_index: 1,
            source: SessionError::Halted(_)
        }
    ));
    Ok(())
}

#[test]
fn halt_exactly_on_the_last_output_step_is_accepted() -> TestResult {
    let mut manager = halting_manager(vec![0x00, 0xff])?;
    let checkpoint = checkpoint_bytes(&mut manager)?;
    let source = fabricated_v1(1, &[1, 2], checkpoint)?;
    let request = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(1))?;
    let outcome =
        chronofork_sessions::edit_movie(&source, &mut manager, &request, &EditMeta::new())?;
    assert_eq!(outcome.movie().step_count(), 2);
    let mut verifier = halting_manager(vec![0x00, 0xff])?;
    let report = outcome.movie().verify(&mut verifier)?;
    assert_eq!(report.steps, 2);
    Ok(())
}

/// Backend family of the test-only payload-size fixture.
const BULK_BACKEND_ID: &str = "chronofork.test.bulk-bounds";
/// Version reported by the bulk fixture backend.
const BULK_BACKEND_VERSION: &str = "0.1.0";
/// Payload format version reported by the bulk fixture backend.
const BULK_FORMAT_VERSION: u16 = 1;
/// Immutable configuration digest of the bulk fixture backend.
fn bulk_config_hash() -> StateHash {
    StateHash::from_bytes([0x5b; 32])
}

/// Test-only deterministic backend with a configurable snapshot payload size.
///
/// It exists only to execute the ADR-0008 size-budget paths that the fixed-size
/// synthetic checkpoint cannot reach: the canonical payload length is
/// `base_payload + growth * steps`, so a capture can exceed
/// [`MAX_ANCHOR_SNAPSHOT_BYTES`] or the cumulative [`MAX_ANCHOR_BYTES`] budget, and
/// one near-`MAX_MOVIE_BYTES` source can drive the v1 writer's output-size check
/// through an edit. The payload padding is not part of the state digest; only the
/// step count, tick and declared length define the state, so the backend remains
/// deterministic and restartable.
#[derive(Clone, Debug)]
struct BulkFactory {
    descriptor: BackendDescriptor,
    base_payload: usize,
    growth: usize,
}

impl BulkFactory {
    /// Creates a factory whose snapshots carry `base_payload + growth * steps` bytes.
    fn new(base_payload: usize, growth: usize) -> Self {
        let capabilities = Capabilities {
            execution: true,
            save_states: true,
            state_cloning: true,
            deterministic_replay: true,
            ..Capabilities::none()
        };
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new(BULK_BACKEND_ID),
                "ChronoFork bulk payload bounds fixture (not an emulator)",
                BULK_BACKEND_VERSION,
                capabilities,
                true,
            ),
            base_payload,
            growth,
        }
    }
}

impl BackendFactory for BulkFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(BulkBackend {
            descriptor: self.descriptor.clone(),
            base_payload: self.base_payload,
            growth: self.growth,
            steps: 0,
            tick: Tick::ZERO,
        }))
    }
}

/// One bulk fixture machine.
#[derive(Debug)]
struct BulkBackend {
    descriptor: BackendDescriptor,
    base_payload: usize,
    growth: usize,
    steps: u64,
    tick: Tick,
}

impl BulkBackend {
    /// Returns the declared canonical payload length at the current step count.
    fn payload_len(&self) -> usize {
        self.growth
            .saturating_mul(usize::try_from(self.steps).unwrap_or(usize::MAX))
            .saturating_add(self.base_payload)
    }

    /// Returns the cheap deterministic state digest of the declared state.
    fn state_hash(&self) -> StateHash {
        let mut body = [0_u8; 16];
        body[..8].copy_from_slice(&self.steps.to_le_bytes());
        body[8..].copy_from_slice(&self.tick.raw().to_le_bytes());
        hash_bytes(&body)
    }
}

impl Backend for BulkBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        // The private inherent digest is the single canonical source for this fixture.
        Ok(BulkBackend::state_hash(self))
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.tick = self.tick.next().map_err(|_| BackendError::TickOverflow)?;
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or(BackendError::TickOverflow)?;
        Ok(StepOutcome {
            tick: self.tick,
            halted: false,
            verified: false,
        })
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        let length = self.payload_len();
        let mut payload = vec![0_u8; length];
        let Some(header) = payload.get_mut(..16) else {
            return Err(BackendError::InvalidSnapshot(
                "bulk payload is shorter than its 16-byte header".to_owned(),
            ));
        };
        header[..8].copy_from_slice(&self.steps.to_le_bytes());
        header[8..].copy_from_slice(&self.tick.raw().to_le_bytes());
        Ok(BackendSnapshot::new(
            BackendId::new(BULK_BACKEND_ID),
            BULK_BACKEND_VERSION,
            BULK_FORMAT_VERSION,
            self.tick,
            bulk_config_hash(),
            self.state_hash(),
            payload,
        ))
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        validate_envelope(
            snapshot,
            self.descriptor.id(),
            BULK_BACKEND_VERSION,
            BULK_FORMAT_VERSION,
            bulk_config_hash(),
        )?;
        let Some(header) = snapshot.payload().get(..16) else {
            return Err(BackendError::InvalidSnapshot(
                "bulk payload is shorter than its 16-byte header".to_owned(),
            ));
        };
        let steps_bytes: [u8; 8] = header[..8].try_into().map_err(|_| {
            BackendError::InvalidSnapshot("bulk payload step field is malformed".to_owned())
        })?;
        let tick_bytes: [u8; 8] = header[8..].try_into().map_err(|_| {
            BackendError::InvalidSnapshot("bulk payload tick field is malformed".to_owned())
        })?;
        let steps = u64::from_le_bytes(steps_bytes);
        let tick = Tick::from_raw(u64::from_le_bytes(tick_bytes));
        let declared = Self {
            descriptor: self.descriptor.clone(),
            base_payload: self.base_payload,
            growth: self.growth,
            steps,
            tick,
        };
        if declared.state_hash() != snapshot.state_hash() {
            return Err(BackendError::InvalidSnapshot(
                "bulk payload state hash mismatch".to_owned(),
            ));
        }
        self.steps = steps;
        self.tick = tick;
        Ok(())
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        Ok(MachineInspection {
            tick: self.tick,
            halted: false,
            frame_index: 0,
            cpu_registers: BTreeMap::new(),
            device_registers: BTreeMap::new(),
            memory: None,
        })
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        Err(BackendError::OutOfBounds {
            address,
            length,
            limit: 0,
        })
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        if data.is_empty() {
            return Ok(());
        }
        Err(BackendError::OutOfBounds {
            address,
            length: data.len(),
            limit: 0,
        })
    }
}

/// Builds a source checkpoint from the bulk fixture backend.
fn bulk_checkpoint(factory: &BulkFactory) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut manager = SessionManager::new(Arc::new(factory.clone()));
    let session = manager.create_session()?;
    let snapshot = manager.snapshot(session)?;
    Ok(manager.snapshot_bytes(session, snapshot)?)
}

/// Builds a structurally valid, anchor-less v2 source over a bulk checkpoint.
///
/// The fabricated records are only used with an edit at step 0, which verifies no
/// prefix; they exist to drive the anchor-capture size paths, not replay.
fn bulk_v2_source(factory: &BulkFactory, steps: usize) -> Result<Movie, Box<dyn Error>> {
    let records = (0..steps)
        .map(|index| {
            MovieStep::new(
                Tick::from_raw(index as u64 + 1),
                InputFrame::new(1),
                StateHash::from_bytes([0x11; 32]),
            )
        })
        .collect();
    Ok(Movie::from_v2_parts(
        meta(1, None)?,
        bulk_checkpoint(factory)?,
        Vec::new(),
        records,
    )?)
}

#[test]
fn rebuild_per_anchor_size_budget_is_a_typed_failure() -> TestResult {
    // The payload grows by 1 MiB per step, so the anchor captured after step 69 of
    // a 70-step source exceeds the 64 MiB per-anchor limit.
    let factory = BulkFactory::new(1024, 1_000_000);
    let source = bulk_v2_source(&factory, 70)?;
    let mut manager = SessionManager::new(Arc::new(factory));
    let request = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(2))?;
    let meta = EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 70 });
    let error = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &meta,
    ))?;
    match error {
        EditError::Movie(MovieError::AnchorSnapshotTooLarge {
            anchor_ordinal: 0,
            limit,
            found,
        }) => {
            assert_eq!(limit, MAX_ANCHOR_SNAPSHOT_BYTES);
            assert!(found > limit as u64);
        }
        other => {
            return Err(format!("expected a per-anchor budget rejection, got {other:?}").into());
        }
    }
    // The import happened, but no partial movie was returned.
    assert_eq!(manager.len(), 1);
    Ok(())
}

#[test]
fn rebuild_cumulative_anchor_budget_is_a_typed_failure() -> TestResult {
    // Two anchors of 25 MiB and 50 MiB pass the per-anchor limit but exceed the
    // 48 MiB cumulative budget at the second capture.
    let factory = BulkFactory::new(1024, 1_000_000);
    let source = bulk_v2_source(&factory, 50)?;
    let mut manager = SessionManager::new(Arc::new(factory));
    let request = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(2))?;
    let meta = EditMeta::new().with_anchor_policy(AnchorPolicy::Rebuild { interval: 25 });
    let error = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &meta,
    ))?;
    match error {
        EditError::Movie(MovieError::AnchorTableTooLarge { limit, found }) => {
            assert_eq!(limit, MAX_ANCHOR_BYTES);
            assert!(found > limit as u64);
        }
        other => {
            return Err(format!("expected a cumulative budget rejection, got {other:?}").into());
        }
    }
    assert_eq!(manager.len(), 1);
    Ok(())
}

#[test]
fn edit_output_size_limit_is_a_typed_failure_without_a_movie() -> TestResult {
    // Derive the v1 fixed overhead plus one 44-byte record from a minimal movie so
    // this fixture does not restate the codec layout.
    let small_factory = BulkFactory::new(64, 0);
    let small_checkpoint = bulk_checkpoint(&small_factory)?;
    let small_source = Movie::from_parts(
        meta(1, None)?,
        small_checkpoint.clone(),
        vec![MovieStep::new(
            Tick::from_raw(1),
            InputFrame::new(1),
            StateHash::from_bytes([0x11; 32]),
        )],
    )?;
    let base = small_source.to_bytes()?.len() - small_checkpoint.len();
    let target_snapshot_len = MAX_MOVIE_BYTES - base;
    let header = small_checkpoint.len() - 64;
    let payload_len = target_snapshot_len - header;

    // A source at exactly MAX_MOVIE_BYTES whose branch adds a one-byte name is the
    // smallest request that makes the single writer reject the output.
    let factory = BulkFactory::new(payload_len, 0);
    let checkpoint = bulk_checkpoint(&factory)?;
    assert_eq!(checkpoint.len(), target_snapshot_len);
    let source = Movie::from_parts(
        meta(1, None)?,
        checkpoint,
        vec![MovieStep::new(
            Tick::from_raw(1),
            InputFrame::new(1),
            StateHash::from_bytes([0x11; 32]),
        )],
    )?;
    // `from_parts` accepted the value, and the arithmetic total is exactly the
    // format limit: fixed overhead plus record plus the target snapshot length.
    assert_eq!(base + source.snapshot_bytes().len(), MAX_MOVIE_BYTES);

    let mut manager = SessionManager::new(Arc::new(factory));
    let request = chronofork_sessions::MovieEdit::new().with_input(0, InputFrame::new(2))?;
    let error = expect_error(chronofork_sessions::edit_movie(
        &source,
        &mut manager,
        &request,
        &EditMeta::new().with_name("x"),
    ))?;
    match error {
        EditError::Movie(MovieError::FileTooLarge { limit, found }) => {
            assert_eq!(limit, MAX_MOVIE_BYTES);
            assert_eq!(found, MAX_MOVIE_BYTES as u64 + 1);
        }
        other => {
            return Err(format!("expected an output-size rejection, got {other:?}").into());
        }
    }
    // Exactly the one imported session exists; no partial movie is returned.
    assert_eq!(manager.len(), 1);
    Ok(())
}
