//! Movie format v2 reader/writer, anchor table and v2 recording integration tests.
//!
//! These tests exercise the public `chronofork_sessions::movie` surface on the
//! deterministic synthetic backend: v1/v2 dispatch through the one reader, v2
//! round-trip and exact canonical size, deterministic bytes, anchor capture and
//! binding, typed rejection of malformed v2 input, and comparison across formats.
//! They return `Result` instead of unwrapping so the workspace `expect_used` lint
//! stays satisfied without a new allowance.

use std::error::Error;
use std::sync::Arc;

use chronofork_backend_api::{BackendId, BackendSnapshot};
use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick, hash_bytes};
use chronofork_sessions::{
    ComparisonVerdict, MAX_ANCHORS, MAX_V2_STEPS, MOVIE_FORMAT_VERSION, MOVIE_INPUT_SCHEMA_VERSION,
    Movie, MovieAnchor, MovieError, MovieMeta, MovieStep, SessionError, SessionManager, Snapshot,
    V2_FIXED_OVERHEAD, V2_MOVIE_FORMAT_VERSION, V2_MOVIE_INPUT_SCHEMA_VERSION, V2_STEP_RECORD_LEN,
    compare_movies,
};
use chronofork_test_backend::SyntheticBackendFactory;

type TestResult = Result<(), Box<dyn Error>>;

const BACKEND_ID: &str = "chronofork.test.synthetic";
const BACKEND_VERSION: &str = "0.1.0";
const NAME: &str = "v2-int";

fn new_manager() -> SessionManager {
    SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
}

fn meta() -> Result<MovieMeta, MovieError> {
    MovieMeta::new(7, Some(NAME.to_owned()), Some(3))
}

fn fabricated_snapshot(tick: u64, state: u8, snapshot_id: u64) -> Snapshot {
    let backend = BackendSnapshot::new(
        BackendId::new(BACKEND_ID),
        BACKEND_VERSION,
        1,
        Tick::from_raw(tick),
        StateHash::from_bytes([0x11; 32]),
        StateHash::from_bytes([state; 32]),
        vec![0xaa, 0xbb, 0xcc, 0xdd],
    );
    Snapshot::new(
        SnapshotId::from_raw(snapshot_id),
        SessionId::from_raw(snapshot_id),
        backend,
    )
}

fn snapshot_bytes(tick: u64, state: u8, snapshot_id: u64) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(fabricated_snapshot(tick, state, snapshot_id).to_bytes()?)
}

fn anchor(
    index: usize,
    tick: u64,
    state: u8,
    snapshot_id: u64,
) -> Result<MovieAnchor, Box<dyn Error>> {
    Ok(MovieAnchor::new(
        index,
        snapshot_bytes(tick, state, snapshot_id)?,
    )?)
}

fn step(tick: u64, input: InputFrame, state: u8) -> MovieStep {
    MovieStep::new(
        Tick::from_raw(tick),
        input,
        StateHash::from_bytes([state; 32]),
    )
}

fn sample_steps() -> Vec<MovieStep> {
    vec![
        step(1, InputFrame::with_devices([1, 0, 0, 0], [0; 4], 0), 0x21),
        step(
            2,
            InputFrame::with_devices([0, 2, 0, 0], [1, -1, 0, 0], 0),
            0x22,
        ),
        step(
            3,
            InputFrame::with_devices([0, 0, 3, 0], [0; 4], 1 << 63),
            0x23,
        ),
        step(
            4,
            InputFrame::with_devices([4, 4, 4, 4], [i16::MIN, i16::MAX, 7, -7], u64::MAX),
            0x24,
        ),
    ]
}

fn sample_movie() -> Result<Movie, Box<dyn Error>> {
    Ok(Movie::from_v2_parts(
        meta()?,
        snapshot_bytes(0, 0x00, 1)?,
        vec![anchor(0, 1, 0x21, 2)?, anchor(2, 3, 0x23, 3)?],
        sample_steps(),
    )?)
}

fn device_inputs() -> Vec<InputFrame> {
    vec![
        InputFrame::with_devices([1, 0, 0, 0], [0, 1, 0, 0], 0),
        InputFrame::with_devices([0, 2, 0, 0], [0, 0, 2, 0], 1),
        InputFrame::new(0),
        InputFrame::with_devices([0, 0, 4, 0], [i16::MIN, 0, 0, 0], 0),
        InputFrame::with_devices([0, 0, 0, 8], [0; 4], 1 << 63),
    ]
}

fn recorded_movie(inputs: &[InputFrame], interval: usize) -> Result<Movie, Box<dyn Error>> {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    for _ in 0..3 {
        manager.step(source, InputFrame::new(3))?;
    }
    Ok(Movie::record_v2(
        &mut manager,
        source,
        meta()?,
        inputs,
        interval,
    )?)
}

fn name_len(movie: &Movie) -> usize {
    movie.meta().name().map_or(0, str::len)
}

fn anchors_len(movie: &Movie) -> usize {
    movie
        .anchors()
        .iter()
        .map(|anchor| 16 + anchor.snapshot_bytes().len())
        .sum()
}

fn anchor_count_offset(movie: &Movie) -> usize {
    22 + name_len(movie) + 8 + 8 + 8 + movie.snapshot_bytes().len()
}

fn anchor_offset(movie: &Movie, ordinal: usize) -> usize {
    let mut offset = anchor_count_offset(movie) + 8;
    for anchor in &movie.anchors()[..ordinal] {
        offset += 16 + anchor.snapshot_bytes().len();
    }
    offset
}

fn step_count_offset(movie: &Movie) -> usize {
    anchor_count_offset(movie) + 8 + anchors_len(movie)
}

fn resign(bytes: &mut [u8]) {
    let body_len = bytes.len() - 32;
    let checksum = hash_bytes(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(checksum.as_bytes());
}

fn mutate(bytes: &[u8], offset: usize, value: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    bytes[offset..offset + value.len()].copy_from_slice(value);
    resign(&mut bytes);
    bytes
}

#[test]
fn one_reader_accepts_v1_and_v2_files() -> TestResult {
    let v1_steps = sample_steps()
        .into_iter()
        .map(|step| {
            MovieStep::new(
                step.outcome_tick(),
                InputFrame::new(step.input().buttons()),
                step.state_hash(),
            )
        })
        .collect();
    let v1 = Movie::from_parts(meta()?, snapshot_bytes(0, 0x00, 1)?, v1_steps)?;
    let v1_bytes = v1.to_bytes()?;
    assert_eq!(&v1_bytes[..8], b"CFMOVIE1");
    assert_eq!(v1.format_version(), MOVIE_FORMAT_VERSION);
    assert_eq!(v1.input_schema_version(), MOVIE_INPUT_SCHEMA_VERSION);
    let decoded_v1 = Movie::from_bytes(&v1_bytes)?;
    assert_eq!(decoded_v1, v1);
    assert!(decoded_v1.anchors().is_empty());
    assert_eq!(decoded_v1.to_bytes()?, v1_bytes);

    // The retained v1 path keeps its own version matrix: a schema-2 or
    // format-2 CFTMOVIE1 file is rejected with the v1 family's supported value.
    assert!(matches!(
        Movie::from_bytes(&mutate(&v1_bytes, 10, &2_u16.to_le_bytes())),
        Err(MovieError::UnsupportedInputSchema {
            found: 2,
            supported: 1
        })
    ));
    assert!(matches!(
        Movie::from_bytes(&mutate(&v1_bytes, 8, &2_u16.to_le_bytes())),
        Err(MovieError::UnsupportedFormatVersion {
            found: 2,
            supported: 1
        })
    ));

    let v2 = sample_movie()?;
    let v2_bytes = v2.to_bytes()?;
    assert_eq!(&v2_bytes[..8], b"CFMOVIE2");
    assert_eq!(v2.format_version(), V2_MOVIE_FORMAT_VERSION);
    assert_eq!(v2.input_schema_version(), V2_MOVIE_INPUT_SCHEMA_VERSION);
    let decoded_v2 = Movie::from_bytes(&v2_bytes)?;
    assert_eq!(decoded_v2, v2);
    assert_eq!(decoded_v2.to_bytes()?, v2_bytes);

    // Metadata, anchors and input schema never decide recorded-expectation
    // equality: the same checkpoint identity and step expectations compare equal
    // across the two formats.
    assert_eq!(
        compare_movies(&v1, &v2).verdict,
        ComparisonVerdict::Identical
    );
    assert_ne!(v1_bytes, v2_bytes);
    Ok(())
}

#[test]
fn v2_round_trip_has_the_exact_canonical_size_and_deterministic_bytes() -> TestResult {
    let movie = sample_movie()?;
    let bytes = movie.to_bytes()?;
    let expected = V2_FIXED_OVERHEAD
        + name_len(&movie)
        + movie.snapshot_bytes().len()
        + anchors_len(&movie)
        + movie.steps().len() * V2_STEP_RECORD_LEN;
    assert_eq!(bytes.len(), expected);
    assert_eq!(name_len(&movie), NAME.len());
    assert_eq!(movie.meta().movie_id(), 7);
    assert_eq!(movie.meta().name(), Some(NAME));
    assert_eq!(movie.meta().parent_movie_id(), Some(3));
    assert_eq!(movie.steps(), sample_steps());
    assert_eq!(movie.anchors().len(), 2);
    assert_eq!(movie.anchors()[0].step_index(), 0);
    assert_eq!(movie.anchors()[1].step_index(), 2);
    assert_eq!(
        movie.anchors()[1].tick(),
        movie.steps()[2].outcome_tick(),
        "anchor tick is the recorded step tick"
    );
    assert_eq!(
        movie.anchors()[1].state_hash(),
        movie.steps()[2].state_hash()
    );

    let rebuilt = sample_movie()?;
    assert_eq!(bytes, rebuilt.to_bytes()?);
    assert_eq!(hash_bytes(&bytes), hash_bytes(&rebuilt.to_bytes()?));
    assert_eq!(Movie::from_bytes(&bytes)?.to_bytes()?, bytes);
    Ok(())
}

#[test]
fn zero_step_v2_movie_round_trips_and_has_no_anchor() -> TestResult {
    let movie = Movie::from_v2_parts(meta()?, snapshot_bytes(4, 0x44, 5)?, Vec::new(), Vec::new())?;
    let bytes = movie.to_bytes()?;
    assert_eq!(
        bytes.len(),
        V2_FIXED_OVERHEAD + name_len(&movie) + movie.snapshot_bytes().len()
    );
    let decoded = Movie::from_bytes(&bytes)?;
    assert_eq!(decoded, movie);
    assert_eq!(decoded.step_count(), 0);
    assert!(decoded.anchors().is_empty());
    assert!(
        decoded
            .anchor_at_or_before(Tick::from_raw(u64::MAX))
            .is_none()
    );
    Ok(())
}

#[test]
fn anchor_selection_returns_the_nearest_anchor_at_or_before_a_tick() -> TestResult {
    let movie = Movie::from_v2_parts(
        meta()?,
        snapshot_bytes(0, 0x00, 1)?,
        vec![
            anchor(0, 1, 0x21, 2)?,
            anchor(2, 3, 0x23, 3)?,
            anchor(4, 5, 0x25, 4)?,
        ],
        vec![
            step(1, InputFrame::new(1), 0x21),
            step(2, InputFrame::new(2), 0x22),
            step(3, InputFrame::new(3), 0x23),
            step(4, InputFrame::new(4), 0x24),
            step(5, InputFrame::new(5), 0x25),
        ],
    )?;
    assert!(movie.anchor_at_or_before(Tick::ZERO).is_none());
    for (tick, expected) in [
        (1_u64, 0_usize),
        (2, 0),
        (3, 2),
        (4, 2),
        (5, 4),
        (u64::MAX, 4),
    ] {
        assert_eq!(
            movie
                .anchor_at_or_before(Tick::from_raw(tick))
                .map(MovieAnchor::step_index),
            Some(expected),
            "nearest anchor for tick {tick}"
        );
    }
    Ok(())
}

#[test]
fn v2_recording_captures_canonical_anchors_and_leaves_the_source_unchanged() -> TestResult {
    let mut manager = new_manager();
    let source = manager.create_session()?;
    for _ in 0..4 {
        manager.step(source, InputFrame::new(5))?;
    }
    let before = manager.inspect(source)?.machine;
    let inputs = device_inputs();
    let movie = Movie::record_v2(&mut manager, source, meta()?, &inputs, 2)?;
    let after = manager.inspect(source)?.machine;
    assert_eq!(after.tick, before.tick);
    assert_eq!(after.state_hash, before.state_hash);
    assert_eq!(movie.format_version(), V2_MOVIE_FORMAT_VERSION);
    assert_eq!(movie.input_schema_version(), V2_MOVIE_INPUT_SCHEMA_VERSION);
    assert_eq!(movie.step_count(), inputs.len());
    assert_eq!(
        movie
            .steps()
            .iter()
            .map(MovieStep::input)
            .collect::<Vec<_>>(),
        inputs
    );
    let indices: Vec<usize> = movie
        .anchors()
        .iter()
        .map(MovieAnchor::step_index)
        .collect();
    assert_eq!(indices, vec![1, 3]);
    for anchor in movie.anchors() {
        let recorded = &movie.steps()[anchor.step_index()];
        assert_eq!(anchor.tick(), recorded.outcome_tick());
        assert_eq!(anchor.state_hash(), recorded.state_hash());
        let snapshot = Snapshot::from_bytes(anchor.snapshot_bytes())?;
        assert_eq!(
            snapshot.to_bytes()?.as_slice(),
            anchor.snapshot_bytes(),
            "an anchor stores the exact canonical Snapshot::to_bytes output"
        );
    }

    let mut verifying = new_manager();
    let report = movie.verify(&mut verifying)?;
    assert_eq!(report.steps, inputs.len());
    assert_eq!(report.final_state_hash, movie.final_state_hash());
    let decoded = Movie::from_bytes(&movie.to_bytes()?)?;
    assert_eq!(decoded, movie);

    // Two independent fresh managers with the same source setup record
    // byte-identical v2 movies.
    let mut second_manager = new_manager();
    let second_source = second_manager.create_session()?;
    for _ in 0..4 {
        second_manager.step(second_source, InputFrame::new(5))?;
    }
    let again = Movie::record_v2(&mut second_manager, second_source, meta()?, &inputs, 2)?;
    assert_eq!(again.to_bytes()?, movie.to_bytes()?);
    Ok(())
}

#[test]
fn v2_anchor_interval_policies_are_deterministic() -> TestResult {
    let inputs = device_inputs();
    let unanchored = recorded_movie(&inputs, 0)?;
    assert!(unanchored.anchors().is_empty());
    let too_wide = recorded_movie(&inputs, inputs.len() + 1)?;
    assert!(too_wide.anchors().is_empty());
    let every_step = recorded_movie(&inputs, 1)?;
    assert_eq!(
        every_step
            .anchors()
            .iter()
            .map(MovieAnchor::step_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );

    let mut manager = new_manager();
    let source = manager.create_session()?;
    assert!(matches!(
        Movie::record_v2(&mut manager, source, meta()?, &[], 2),
        Err(MovieError::InvalidAnchorInterval { interval: 2 })
    ));
    let zero_step = Movie::record_v2(&mut manager, source, meta()?, &[], 0)?;
    assert_eq!(zero_step.step_count(), 0);
    assert!(zero_step.anchors().is_empty());
    Ok(())
}

#[test]
fn v2_anchor_bindings_are_rejected_through_the_public_constructors() -> TestResult {
    let embedded = snapshot_bytes(0, 0x00, 1)?;
    let steps = sample_steps();

    assert!(matches!(
        Movie::from_v2_parts(
            meta()?,
            embedded.clone(),
            vec![anchor(1, 2, 0x22, 2)?, anchor(1, 2, 0x22, 3)?],
            steps.clone()
        ),
        Err(MovieError::AnchorIndexNotIncreasing {
            anchor_ordinal: 1,
            previous: 1,
            found: 1
        })
    ));
    assert!(matches!(
        Movie::from_v2_parts(
            meta()?,
            embedded.clone(),
            vec![anchor(4, 5, 0x25, 2)?],
            steps.clone()
        ),
        Err(MovieError::AnchorIndexOutOfRange {
            anchor_ordinal: 0,
            index: 4,
            step_count: 4
        })
    ));
    assert!(matches!(
        Movie::from_v2_parts(
            meta()?,
            embedded.clone(),
            vec![anchor(0, 1, 0x21, 2)?],
            Vec::new()
        ),
        Err(MovieError::AnchorIndexOutOfRange {
            anchor_ordinal: 0,
            index: 0,
            step_count: 0
        })
    ));
    assert!(matches!(
        Movie::from_v2_parts(
            meta()?,
            embedded.clone(),
            vec![anchor(0, 2, 0x22, 2)?],
            steps.clone()
        ),
        Err(MovieError::AnchorTickMismatch {
            anchor_ordinal: 0,
            index: 0,
            ..
        })
    ));
    assert!(matches!(
        Movie::from_v2_parts(meta()?, embedded, vec![anchor(0, 1, 0xee, 2)?], steps),
        Err(MovieError::AnchorStateHashMismatch {
            anchor_ordinal: 0,
            index: 0,
            ..
        })
    ));

    let mut corrupt = snapshot_bytes(1, 0x21, 2)?;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    assert!(matches!(
        MovieAnchor::new(0, corrupt),
        Err(MovieError::InvalidAnchorSnapshot {
            anchor_ordinal: 0,
            source: SessionError::InvalidSnapshot(_)
        })
    ));
    Ok(())
}

#[test]
fn malformed_v2_input_is_typed_and_never_panics() -> TestResult {
    let movie = sample_movie()?;
    let bytes = movie.to_bytes()?;

    assert!(matches!(
        Movie::from_bytes(&mutate(&bytes, 0, b"X")),
        Err(MovieError::BadMagic)
    ));
    assert!(matches!(
        Movie::from_bytes(&mutate(&bytes, 8, &1_u16.to_le_bytes())),
        Err(MovieError::UnsupportedFormatVersion {
            found: 1,
            supported: 2
        })
    ));
    assert!(matches!(
        Movie::from_bytes(&mutate(&bytes, 10, &1_u16.to_le_bytes())),
        Err(MovieError::UnsupportedInputSchema {
            found: 1,
            supported: 2
        })
    ));
    assert!(matches!(
        Movie::from_bytes(&mutate(&bytes, 0, b"CFMOVIE1")),
        Err(MovieError::UnsupportedFormatVersion {
            found: 2,
            supported: 1
        })
    ));
    let mut flipped = bytes.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0xff;
    assert!(matches!(
        Movie::from_bytes(&flipped),
        Err(MovieError::ChecksumMismatch)
    ));

    assert!(matches!(
        Movie::from_bytes(&mutate(
            &bytes,
            step_count_offset(&movie),
            &(MAX_V2_STEPS as u64 + 1).to_le_bytes()
        )),
        Err(MovieError::TooManySteps { limit, .. }) if limit == MAX_V2_STEPS
    ));
    assert!(matches!(
        Movie::from_bytes(&mutate(
            &bytes,
            anchor_count_offset(&movie),
            &(MAX_ANCHORS as u64 + 1).to_le_bytes()
        )),
        Err(MovieError::TooManyAnchors { limit, .. }) if limit == MAX_ANCHORS
    ));
    let second_anchor = anchor_offset(&movie, 1);
    assert!(matches!(
        Movie::from_bytes(&mutate(&bytes, second_anchor, &0_u64.to_le_bytes())),
        Err(MovieError::AnchorIndexNotIncreasing {
            anchor_ordinal: 1,
            ..
        })
    ));

    for cut in 0..bytes.len() {
        assert!(
            Movie::from_bytes(&bytes[..cut]).is_err(),
            "truncated at {cut} bytes must be rejected"
        );
    }
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for length in 0..=1024 {
        let mut random = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            random.push((state >> 24).to_le_bytes()[0]);
        }
        assert!(
            Movie::from_bytes(&random).is_err(),
            "arbitrary {length}-byte movie must be rejected"
        );
    }
    Ok(())
}
