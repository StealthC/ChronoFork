//! Movie record, replay and verification integration tests.
//!
//! These tests exercise the public movie API on the deterministic synthetic
//! backend: recording from a source session, replaying in fresh managers,
//! exact first-divergence reporting, restart/reopen from a durable checkpoint,
//! branch metadata and per-movie isolation. They return `Result` instead of
//! unwrapping so the workspace `expect_used` lint stays satisfied without a new
//! file-level allowance.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_core::{InputFrame, SessionId, StateHash, Tick};
use chronofork_sessions::{
    DurableLineage, Movie, MovieError, MovieMeta, MovieStep, SessionError, SessionManager,
    Snapshot, SnapshotRepository,
};
use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

type TestResult = Result<(), Box<dyn Error>>;

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn meta(movie_id: u64, name: Option<&str>, parent: Option<u64>) -> Result<MovieMeta, MovieError> {
    MovieMeta::new(movie_id, name.map(str::to_owned), parent)
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

fn flip_hash(hash: StateHash) -> StateHash {
    let mut bytes = *hash.as_bytes();
    bytes[0] ^= 0xff;
    StateHash::from_bytes(bytes)
}

fn temp_root(name: &str) -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir().join(format!("chronofork-movie-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn list_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[test]
fn record_and_verify_reproduce_every_tick_and_state_hash() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 3, 5)?;
    let source_before = machine(&manager, source)?;

    let recorded = recorded_inputs();
    let movie = Movie::record(
        &mut manager,
        source,
        meta(1, Some("baseline"), None)?,
        &recorded,
    )?;

    assert_eq!(movie.start_tick(), source_before.0);
    let embedded = Snapshot::from_bytes(movie.snapshot_bytes())?;
    assert_eq!(embedded.tick(), source_before.0);
    assert_eq!(embedded.state_hash(), source_before.1);
    assert_eq!(movie.step_count(), recorded.len());
    assert_eq!(machine(&manager, source)?, source_before);
    for (index, step) in movie.steps().iter().enumerate() {
        assert_eq!(step.input(), recorded[index]);
        assert_eq!(
            step.outcome_tick(),
            Tick::from_raw(source_before.0.raw() + index as u64 + 1)
        );
    }
    let last_step = movie
        .steps()
        .last()
        .ok_or_else(|| "recording must have at least one step".to_owned())?;
    assert_eq!(movie.end_tick(), last_step.outcome_tick());
    assert_eq!(movie.final_state_hash(), last_step.state_hash());

    // Replay in a manager that already owns an unrelated session: verification
    // registers only its own imported session and never touches the bystander.
    let mut verifying = new_manager();
    let bystander = verifying.create_session()?;
    step_many(&mut verifying, bystander, 9, 2)?;
    let bystander_before = machine(&verifying, bystander)?;
    let report = movie.verify(&mut verifying)?;
    assert_eq!(machine(&verifying, bystander)?, bystander_before);
    assert_eq!(verifying.len(), 2);

    assert_eq!(report.start_tick, movie.start_tick());
    assert_eq!(report.end_tick, movie.end_tick());
    assert_eq!(report.steps, recorded.len());
    assert_eq!(report.final_tick, movie.end_tick());
    assert_eq!(report.final_state_hash, movie.final_state_hash());
    println!(
        "replay: start_tick={} end_tick={} steps={} final_tick={} final_state_hash={}",
        report.start_tick,
        report.end_tick,
        report.steps,
        report.final_tick,
        report.final_state_hash
    );

    // A separate manager and a second run reach the same report.
    let second_report = movie.verify(&mut new_manager())?;
    assert_eq!(second_report, report);

    // Canonical bytes round-trip and the source stays unchanged.
    let decoded = Movie::from_bytes(&movie.to_bytes()?)?;
    assert_eq!(decoded, movie);
    assert_eq!(machine(&manager, source)?, source_before);
    Ok(())
}

#[test]
fn zero_step_movie_verifies_the_embedded_checkpoint() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 4, 7)?;
    let before = machine(&manager, source)?;

    let movie = Movie::record(&mut manager, source, meta(2, None, None)?, &[])?;
    assert_eq!(movie.step_count(), 0);
    assert_eq!(movie.start_tick(), before.0);
    assert_eq!(movie.end_tick(), before.0);
    assert_eq!(movie.final_state_hash(), before.1);
    assert_eq!(machine(&manager, source)?, before);

    let report = movie.verify(&mut new_manager())?;
    assert_eq!(report.start_tick, before.0);
    assert_eq!(report.end_tick, before.0);
    assert_eq!(report.steps, 0);
    assert_eq!(report.final_tick, before.0);
    assert_eq!(report.final_state_hash, before.1);
    Ok(())
}

#[test]
fn first_divergence_is_exact_and_stops_at_the_first_mismatch() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 1, 3)?;
    let source_before = machine(&manager, source)?;
    let movie = Movie::record(
        &mut manager,
        source,
        meta(3, Some("divergence"), None)?,
        &recorded_inputs(),
    )?;
    let first = 3_usize;

    let mut steps = movie.steps().to_vec();
    let original_hash = steps[first].state_hash();
    let altered_hash = flip_hash(original_hash);
    steps[first] = MovieStep::new(
        steps[first].outcome_tick(),
        steps[first].input(),
        altered_hash,
    );
    let divergent =
        Movie::from_parts(movie.meta().clone(), movie.snapshot_bytes().to_vec(), steps)?;

    let error = match divergent.verify(&mut new_manager()) {
        Ok(_) => return Err("the altered movie must diverge".to_owned().into()),
        Err(error) => error,
    };
    match error {
        MovieError::Divergence {
            tick,
            input_index,
            expected,
            observed,
        } => {
            assert_eq!(tick, movie.steps()[first].outcome_tick());
            assert_eq!(input_index, first);
            assert_eq!(expected, altered_hash);
            assert_eq!(observed, original_hash);
            println!(
                "divergence: tick={tick} input_index={input_index} expected={expected} observed={observed}"
            );
        }
        other => return Err(format!("expected a state divergence, got {other}").into()),
    }

    // An intact movie reports no divergence.
    assert!(movie.verify(&mut new_manager()).is_ok());

    // Altering only a later step reports that later index, not the earlier one.
    let later = first + 2;
    let mut later_steps = movie.steps().to_vec();
    let later_original = later_steps[later].state_hash();
    let later_altered = flip_hash(later_original);
    later_steps[later] = MovieStep::new(
        later_steps[later].outcome_tick(),
        later_steps[later].input(),
        later_altered,
    );
    let later_movie = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        later_steps,
    )?;
    match later_movie.verify(&mut new_manager()) {
        Err(MovieError::Divergence {
            tick,
            input_index,
            expected,
            observed,
        }) => {
            assert_eq!(input_index, later);
            assert_eq!(tick, movie.steps()[later].outcome_tick());
            assert_eq!(expected, later_altered);
            assert_eq!(observed, later_original);
        }
        other => {
            return Err(format!("expected divergence at index {later}, got {other:?}").into());
        }
    }

    // Altering two steps reports only the first: replay stops at the first
    // mismatch and never reports a later one.
    let mut both_steps = movie.steps().to_vec();
    both_steps[first] = MovieStep::new(
        both_steps[first].outcome_tick(),
        both_steps[first].input(),
        altered_hash,
    );
    both_steps[later] = MovieStep::new(
        both_steps[later].outcome_tick(),
        both_steps[later].input(),
        later_altered,
    );
    let both_movie = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        both_steps,
    )?;
    match both_movie.verify(&mut new_manager()) {
        Err(MovieError::Divergence { input_index, .. }) => assert_eq!(input_index, first),
        other => {
            return Err(format!("expected the first divergence, got {other:?}").into());
        }
    }

    // A changed recorded tick without a changed hash is the typed tick mismatch.
    let last = movie
        .steps()
        .len()
        .checked_sub(1)
        .ok_or_else(|| "expected steps".to_owned())?;
    let mut tick_steps = movie.steps().to_vec();
    let original_tick = tick_steps[last].outcome_tick();
    let altered_tick = Tick::from_raw(original_tick.raw() + 5);
    tick_steps[last] = MovieStep::new(
        altered_tick,
        tick_steps[last].input(),
        tick_steps[last].state_hash(),
    );
    let tick_movie = Movie::from_parts(
        movie.meta().clone(),
        movie.snapshot_bytes().to_vec(),
        tick_steps,
    )?;
    match tick_movie.verify(&mut new_manager()) {
        Err(MovieError::TickMismatch {
            input_index,
            expected,
            observed,
        }) => {
            assert_eq!(input_index, last);
            assert_eq!(expected, altered_tick);
            assert_eq!(observed, original_tick);
        }
        other => return Err(format!("expected a tick mismatch, got {other:?}").into()),
    }

    // The source is unchanged after all of the replay attempts.
    assert_eq!(machine(&manager, source)?, source_before);
    Ok(())
}

#[test]
fn restart_reopen_complement_records_from_a_durable_checkpoint() -> TestResult {
    let root = temp_root("restart")?;
    let repository = SnapshotRepository::initialize(&root)?;
    let mut manager = new_manager();
    let runtime = manager.create_session()?;
    step_many(&mut manager, runtime, 5, 6)?;
    let session = repository.create_session(
        SessionId::from_raw(runtime.raw()),
        Some("movie-source"),
        DurableLineage::root(),
    )?;
    manager.attach_storage_session(runtime, session.session_key)?;
    let snapshot_id = manager.snapshot(runtime)?;
    let record = manager.snapshot_record(runtime, snapshot_id)?;
    let entry = repository.save_snapshot(session.session_key, &record, Some("checkpoint"))?;
    let checkpoint_path = root.join(format!(
        "v1/sessions/session-{}/snapshots/snapshot-{}/checkpoint.cfsnap",
        session.session_key, entry.snapshot_key
    ));
    let checkpoint_before = std::fs::read(&checkpoint_path)?;
    drop(manager);
    drop(repository);

    // Reopen the durable store, rehydrate the committed checkpoint and record
    // from it without writing storage.
    let repository = SnapshotRepository::open(&root)?;
    let loaded = repository.load_snapshot(session.session_key, entry.snapshot_key)?;
    let durable_snapshot = loaded.snapshot.clone();
    let files_before = list_files(&root)?;
    let mut recording_manager = new_manager();
    let source = recording_manager
        .create_session_from_stored_snapshot(loaded.snapshot, session.session_key)?;
    let movie = Movie::record(
        &mut recording_manager,
        source,
        meta(4, Some("durable-seeded"), None)?,
        &recorded_inputs(),
    )?;
    // Recording captures a fresh canonical envelope through the existing
    // checkpoint path; its identity bytes are new, but it represents the exact
    // durable emulated state.
    let embedded = Snapshot::from_bytes(movie.snapshot_bytes())?;
    assert_eq!(embedded.tick(), durable_snapshot.tick());
    assert_eq!(embedded.state_hash(), durable_snapshot.state_hash());
    assert_eq!(
        embedded.backend().payload(),
        durable_snapshot.backend().payload()
    );
    assert_eq!(
        list_files(&root)?,
        files_before,
        "recording must not write storage"
    );
    assert_eq!(std::fs::read(&checkpoint_path)?, checkpoint_before);
    drop(recording_manager);
    drop(repository);

    // A third manager replays only the movie's embedded checkpoint.
    let mut verifying_manager = new_manager();
    let report = movie.verify(&mut verifying_manager)?;
    let last = movie
        .steps()
        .last()
        .ok_or_else(|| "expected recorded steps".to_owned())?;
    assert_eq!(report.start_tick, movie.start_tick());
    assert_eq!(report.end_tick, last.outcome_tick());
    assert_eq!(report.steps, movie.step_count());
    assert_eq!(report.final_tick, last.outcome_tick());
    assert_eq!(report.final_state_hash, last.state_hash());
    println!(
        "restart replay: start_tick={} end_tick={} steps={} final_tick={} final_state_hash={}",
        report.start_tick,
        report.end_tick,
        report.steps,
        report.final_tick,
        report.final_state_hash
    );
    Ok(())
}

#[test]
fn branch_metadata_round_trips_and_two_movies_stay_isolated() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    step_many(&mut manager, source, 6, 4)?;
    let source_before = machine(&manager, source)?;
    let recorded = recorded_inputs();

    let first = Movie::record(
        &mut manager,
        source,
        meta(7, Some("branch-a"), Some(3))?,
        &recorded,
    )?;
    let second = Movie::record(&mut manager, source, meta(8, None, None)?, &recorded)?;

    // A dangling parent is accepted as passive metadata and never resolved.
    assert_eq!(first.meta().parent_movie_id(), Some(3));
    assert_eq!(first.meta().name(), Some("branch-a"));
    assert_eq!(second.meta().name(), None);
    assert_eq!(second.meta().parent_movie_id(), None);

    let first_bytes = first.to_bytes()?;
    let second_bytes = second.to_bytes()?;
    assert_ne!(first_bytes, second_bytes);
    let decoded = Movie::from_bytes(&first_bytes)?;
    assert_eq!(decoded, first);

    // A metadata-only change alters bytes but not the recorded execution.
    assert_eq!(first.start_tick(), second.start_tick());
    assert_eq!(first.end_tick(), second.end_tick());
    assert_eq!(first.step_count(), second.step_count());
    assert_eq!(first.final_state_hash(), second.final_state_hash());
    assert_eq!(first.steps(), second.steps());

    // Each movie verifies independently and neither rewrites the other.
    let first_report = first.verify(&mut new_manager())?;
    let second_report = second.verify(&mut new_manager())?;
    assert_eq!(first_report, second_report);
    assert_eq!(first.to_bytes()?, first_bytes);
    assert_eq!(second.to_bytes()?, second_bytes);
    assert_eq!(machine(&manager, source)?, source_before);
    Ok(())
}

#[test]
fn different_checkpoints_embed_different_snapshot_bytes() -> TestResult {
    let mut manager = new_manager();
    let first_session = manager.create_session()?;
    step_many(&mut manager, first_session, 1, 4)?;
    let second_session = manager.create_session()?;
    step_many(&mut manager, second_session, 2, 9)?;

    let first = Movie::record(&mut manager, first_session, meta(1, None, None)?, &[])?;
    let second = Movie::record(&mut manager, second_session, meta(2, None, None)?, &[])?;
    assert_ne!(first.snapshot_bytes(), second.snapshot_bytes());
    assert_ne!(first.to_bytes()?, second.to_bytes()?);
    assert_ne!(first.start_tick(), second.start_tick());
    assert!(first.verify(&mut new_manager()).is_ok());
    assert!(second.verify(&mut new_manager()).is_ok());
    Ok(())
}

#[test]
fn replay_import_maps_backend_incompatibility_without_a_second_predicate() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    let recorded = recorded_inputs();
    let movie = Movie::record(&mut manager, source, meta(1, None, None)?, &recorded[..3])?;

    // A different synthetic program changes the configuration hash; the existing
    // import path must decide incompatibility and the movie maps it unchanged.
    let other = SyntheticBackendFactory::new(SyntheticProgram::new(vec![0x01, 0x06, 0x00, 0x00])?);
    let mut other_manager = SessionManager::new(Arc::new(other));
    let error = match movie.verify(&mut other_manager) {
        Ok(_) => return Err("a foreign configuration must not verify".to_owned().into()),
        Err(error) => error,
    };
    match error {
        MovieError::SnapshotImport(SessionError::IncompatibleSnapshot(_)) => {}
        other => {
            return Err(format!("expected a mapped incompatibility, got {other}").into());
        }
    }
    Ok(())
}

#[test]
fn a_failed_recording_is_typed_and_leaves_the_source_unchanged() -> TestResult {
    let halt_factory = SyntheticBackendFactory::new(SyntheticProgram::new(vec![0xff])?);
    let mut manager = SessionManager::new(Arc::new(halt_factory));
    let source = manager.create_session()?;
    let before = machine(&manager, source)?;
    let two_inputs = [InputFrame::NEUTRAL, InputFrame::NEUTRAL];
    let error = match Movie::record(&mut manager, source, meta(1, None, None)?, &two_inputs) {
        Ok(_) => {
            return Err("recording must fail when the backend halts"
                .to_owned()
                .into());
        }
        Err(error) => error,
    };
    match error {
        MovieError::RecordFailed {
            input_index,
            source: SessionError::Halted(_),
        } => {
            assert_eq!(input_index, 0);
        }
        other => return Err(format!("expected a halted recording, got {other}").into()),
    }
    assert_eq!(machine(&manager, source)?, before);
    Ok(())
}
