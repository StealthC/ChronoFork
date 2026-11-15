//! Canonical `.cfmovie` container, recorder and replay verifier.
//!
//! This module implements the v1 contract of
//! [ADR-0004](../../../../docs/adr/0004-tas-movie-format.md): one self-contained
//! file that embeds the exact canonical checkpoint bytes, the ordered
//! backend-defined input sequence and a per-step expectation record, followed by
//! one trailing SHA-256.
//!
//! There is exactly one reader ([`Movie::from_bytes`]) and one writer
//! ([`Movie::to_bytes`]), both built on [`chronofork_core::codec`]. The embedded
//! checkpoint is encoded only by [`Snapshot::to_bytes`] and decoded only by
//! [`Snapshot::from_bytes`]; backend and configuration compatibility are decided
//! only by the existing import path
//! [`SessionManager::create_session_from_snapshot_bytes`], never here.
//!
//! Recording captures the source checkpoint and executes the inputs in a separate
//! imported session, so the source tick and state are never advanced. Replay
//! imports the embedded checkpoint into a fresh session and stops at the first
//! mismatch, reporting its exact recorded tick, input index, expected hash and
//! observed hash.
//!
//! This module keeps the single v1 codec, recorder and validating constructor.
//! Ranged replay over a validated tick window and comparison of two movies'
//! recorded expectations are owned by [`crate::replay`]; [`Movie::verify`] is the
//! full range of that same replay path, so the checkpoint import and the
//! stepping/verification loop exist exactly once.
//!
//! Deferred (not implemented and not claimed): multi-controller/ports/analog
//! input, checkpoint anchors and a movie format v2, an in-process rewind/state
//! cache, interactive stepping, rerecord, branch editing, checkpoint-reference
//! movies, import/export adapters and real-hardware input TAS.

use std::error::Error;
use std::fmt;

use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{InputFrame, SessionId, StateHash, Tick, hash_bytes};

use crate::error::SessionError;
use crate::manager::SessionManager;
use crate::replay::ReplayRange;
use crate::snapshot::Snapshot;

/// Version of the canonical movie container.
pub const MOVIE_FORMAT_VERSION: u16 = 1;

/// Version of the recorded input schema: one backend-defined `u32` mask per step.
pub const MOVIE_INPUT_SCHEMA_VERSION: u16 = 1;

/// Maximum size of one complete movie file, including the trailing checksum.
///
/// A movie larger than this is rejected by the reader before any body is parsed,
/// and the writer refuses to produce one.
pub const MAX_MOVIE_BYTES: usize = 256 * 1024 * 1024;

/// Maximum number of recorded steps in one movie.
///
/// The bound and [`MAX_MOVIE_BYTES`] are consistent: `MAX_STEPS × 44` bytes is
/// 184,549,376 bytes, leaving the remaining file budget for metadata and the
/// embedded checkpoint.
pub const MAX_STEPS: usize = 4_194_304;

/// Maximum number of UTF-8 bytes in the optional movie name.
///
/// The v1 format fixes this value at 256 bytes. It reuses the session-layer name
/// bound so the workspace has one numeric source, but a future change to that
/// constant must not change the v1 movie reader until the movie format version is
/// bumped with updated rejection tests.
pub const MAX_NAME_BYTES: usize = crate::storage::MAX_NAME_BYTES;

const MAGIC: [u8; 8] = *b"CFMOVIE1";
const CHECKSUM_LEN: usize = 32;
const STEP_RECORD_LEN: usize = 8 + 4 + 32;
/// Fixed container overhead before the optional name, checkpoint and steps.
///
/// `8 magic + 2 format + 2 schema + 8 movie id + 2 name length + 8 parent id`
/// `+ 8 start tick + 8 snapshot length + 8 step count + 32 checksum = 86`.
const FIXED_OVERHEAD: usize = MAGIC.len()
    + 2  // movie format version
    + 2  // input schema version
    + 8  // movie id
    + 2  // name length
    + 8  // parent movie id
    + 8  // start tick
    + 8  // embedded snapshot length
    + 8  // step count
    + CHECKSUM_LEN;

/// One recorded step: the applied input and its expected outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MovieStep {
    outcome_tick: Tick,
    input: InputFrame,
    state_hash: StateHash,
}

impl MovieStep {
    /// Creates a step record.
    ///
    /// The validating constructor [`Movie::from_parts`] checks the resulting tick
    /// sequence before a movie can be encoded.
    #[must_use]
    pub const fn new(outcome_tick: Tick, input: InputFrame, state_hash: StateHash) -> Self {
        Self {
            outcome_tick,
            input,
            state_hash,
        }
    }

    /// Returns the tick the backend reported after applying this input.
    #[must_use]
    pub const fn outcome_tick(&self) -> Tick {
        self.outcome_tick
    }

    /// Returns the applied backend-defined input mask.
    #[must_use]
    pub const fn input(&self) -> InputFrame {
        self.input
    }

    /// Returns the expected state digest after this step.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }
}

/// Non-canonical movie labels that never determine emulated state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovieMeta {
    movie_id: u64,
    name: Option<String>,
    parent_movie_id: Option<u64>,
}

impl MovieMeta {
    /// Validates and creates movie metadata.
    ///
    /// `movie_id` must be non-zero. The optional name must be non-empty, valid
    /// UTF-8 by construction, at most [`MAX_NAME_BYTES`] bytes and free of control
    /// characters. The optional parent id must be non-zero and differ from
    /// `movie_id`; a dangling parent is accepted as passive provenance metadata
    /// and is never resolved.
    pub fn new(
        movie_id: u64,
        name: Option<String>,
        parent_movie_id: Option<u64>,
    ) -> Result<Self, MovieError> {
        if movie_id == 0 {
            return Err(MovieError::InvalidMovieId);
        }
        if let Some(parent) = parent_movie_id {
            if parent == 0 {
                return Err(MovieError::InvalidParentMovieId);
            }
            if parent == movie_id {
                return Err(MovieError::InvalidParentMovieId);
            }
        }
        if let Some(name) = &name {
            validate_name(name)?;
        }
        Ok(Self {
            movie_id,
            name,
            parent_movie_id,
        })
    }

    /// Returns the caller-supplied logical movie id.
    #[must_use]
    pub const fn movie_id(&self) -> u64 {
        self.movie_id
    }

    /// Returns the optional non-canonical name.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the optional passive parent movie id.
    #[must_use]
    pub const fn parent_movie_id(&self) -> Option<u64> {
        self.parent_movie_id
    }
}

/// A validated v1 movie.
///
/// Values are only produced by [`Movie::from_bytes`] (decoding),
/// [`Movie::from_parts`] (validating construction) or [`Movie::record`]
/// (recording). Fields are private; a divergent movie for a test is built through
/// the same validating constructor with a deliberately altered step record and a
/// freshly computed trailing checksum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Movie {
    meta: MovieMeta,
    snapshot_bytes: Vec<u8>,
    start_tick: Tick,
    start_state_hash: StateHash,
    steps: Vec<MovieStep>,
}

impl Movie {
    /// Builds a movie from validated metadata, a canonical snapshot and steps.
    ///
    /// The snapshot bytes must be exactly [`Snapshot::to_bytes`] output for a
    /// compatible checkpoint; they are decoded once by [`Snapshot::from_bytes`]
    /// to derive the start tick and state digest. The steps must be strictly
    /// increasing and start after the checkpoint tick. The value is rejected when
    /// it would exceed [`MAX_STEPS`] or [`MAX_MOVIE_BYTES`]. This is the single
    /// validating construction path; it delegates encoding to [`Movie::to_bytes`].
    pub fn from_parts(
        meta: MovieMeta,
        snapshot_bytes: Vec<u8>,
        steps: Vec<MovieStep>,
    ) -> Result<Self, MovieError> {
        let snapshot =
            Snapshot::from_bytes(&snapshot_bytes).map_err(MovieError::InvalidEmbeddedSnapshot)?;
        validate_steps(&steps, snapshot.tick())?;
        let movie = Self {
            meta,
            snapshot_bytes,
            start_tick: snapshot.tick(),
            start_state_hash: snapshot.state_hash(),
            steps,
        };
        // Enforce the size bounds through the single writer before the value can
        // be handed out.
        movie.to_bytes()?;
        Ok(movie)
    }

    /// Records an input sequence from `source` without advancing it.
    ///
    /// The source checkpoint is captured through the existing checkpoint path and
    /// imported into a separate fresh session; the inputs are applied to that
    /// imported session and every outcome becomes an expectation record. The
    /// source session's tick, state digest and mutable state are unchanged (the
    /// checkpoint record stored by [`SessionManager::snapshot`] is the only
    /// permitted source-side effect). A failed or prematurely halted step yields a
    /// typed error and no movie.
    pub fn record(
        manager: &mut SessionManager,
        source: SessionId,
        meta: MovieMeta,
        inputs: &[InputFrame],
    ) -> Result<Self, MovieError> {
        if inputs.len() > MAX_STEPS {
            return Err(MovieError::TooManySteps {
                limit: MAX_STEPS,
                found: inputs.len() as u64,
            });
        }
        let snapshot_id = manager
            .snapshot(source)
            .map_err(MovieError::SnapshotCapture)?;
        let snapshot_bytes = manager
            .snapshot_bytes(source, snapshot_id)
            .map_err(MovieError::SnapshotCapture)?;
        let session = manager
            .create_session_from_snapshot_bytes(&snapshot_bytes)
            .map_err(MovieError::SnapshotImport)?;
        let mut steps = Vec::with_capacity(inputs.len());
        for (input_index, input) in inputs.iter().enumerate() {
            let outcome =
                manager
                    .step(session, *input)
                    .map_err(|source| MovieError::RecordFailed {
                        input_index,
                        source,
                    })?;
            if outcome.halted && input_index + 1 < inputs.len() {
                return Err(MovieError::RecordFailed {
                    input_index,
                    source: SessionError::Halted(session),
                });
            }
            steps.push(MovieStep::new(outcome.tick, *input, outcome.state_hash));
        }
        Self::from_parts(meta, snapshot_bytes, steps)
    }

    /// Replays the movie in a fresh imported session and verifies every step.
    ///
    /// This is the full range `[start_tick, end_tick]` of the single ranged
    /// replay path in [`crate::replay`]: the embedded checkpoint is imported
    /// through the existing
    /// [`SessionManager::create_session_from_snapshot_bytes`] path, so backend,
    /// payload and configuration compatibility stay owned by the existing
    /// compatibility predicate. The first mismatching step is reported as
    /// [`MovieError::TickMismatch`] or [`MovieError::Divergence`] with its exact
    /// recorded tick, input index, expected value and observed value; replay never
    /// resynchronizes and never reports a later mismatch instead. A pre-existing
    /// session in `manager` is never touched.
    pub fn verify(&self, manager: &mut SessionManager) -> Result<ReplayReport, MovieError> {
        let outcome = ReplayRange::full(self).replay(manager)?;
        Ok(ReplayReport {
            start_tick: outcome.start_tick,
            end_tick: outcome.end_tick,
            steps: outcome.window_steps,
            final_tick: outcome.final_tick,
            final_state_hash: outcome.final_state_hash,
        })
    }

    /// Encodes the movie into its canonical, checksummed bytes.
    ///
    /// The output is deterministic: it contains only fixed-width little-endian
    /// fields, the optional name bytes, the verbatim checkpoint bytes and the step
    /// records. Host time, paths and other host-dependent values never enter the
    /// encoding.
    pub fn to_bytes(&self) -> Result<Vec<u8>, MovieError> {
        let name_len = self.meta.name.as_ref().map_or(0, String::len);
        let total = checked_movie_len(name_len, self.snapshot_bytes.len(), self.steps.len())?;
        let mut output = Writer::with_capacity(total);
        output.write_bytes(&MAGIC);
        output.write_u16_le(MOVIE_FORMAT_VERSION);
        output.write_u16_le(MOVIE_INPUT_SCHEMA_VERSION);
        output.write_u64_le(self.meta.movie_id);
        // `checked_movie_len` rejected more than `MAX_NAME_BYTES` (256), so the
        // conversion is exact.
        output.write_u16_le(name_len as u16);
        if let Some(name) = &self.meta.name {
            output.write_bytes(name.as_bytes());
        }
        output.write_u64_le(self.meta.parent_movie_id.unwrap_or(0));
        output.write_u64_le(self.start_tick.raw());
        // The size bound is checked first, so this length fits both the field and
        // the address space on every supported platform.
        output.write_u64_le(self.snapshot_bytes.len() as u64);
        output.write_bytes(&self.snapshot_bytes);
        output.write_u64_le(self.steps.len() as u64);
        for step in &self.steps {
            output.write_u64_le(step.outcome_tick.raw());
            output.write_u32_le(step.input.buttons());
            output.write_bytes(step.state_hash.as_bytes());
        }
        let checksum = hash_bytes(output.as_slice());
        output.write_bytes(checksum.as_bytes());
        Ok(output.into_bytes())
    }

    /// Decodes and validates canonical movie bytes.
    ///
    /// The reader is a pure, bounded byte parser: it performs no I/O and executes
    /// no backend. Declared lengths and counts are validated against the v1 limits
    /// and the bytes that actually remain before any buffer is allocated; the
    /// trailing checksum is verified before any field is trusted; trailing bytes
    /// are rejected. Arbitrary input never panics.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MovieError> {
        validate_slice_len(bytes.len())?;
        let body_len = bytes.len() - CHECKSUM_LEN;
        let (body, checksum) = bytes.split_at(body_len);
        if hash_bytes(body).as_bytes() != checksum {
            return Err(MovieError::ChecksumMismatch);
        }

        let mut reader = Reader::new(body);
        let magic = reader.read_array::<8>().map_err(codec_error)?;
        if magic != MAGIC {
            return Err(MovieError::BadMagic);
        }
        let format_version = reader.read_u16_le().map_err(codec_error)?;
        if format_version != MOVIE_FORMAT_VERSION {
            return Err(MovieError::UnsupportedFormatVersion {
                found: format_version,
                supported: MOVIE_FORMAT_VERSION,
            });
        }
        let input_schema = reader.read_u16_le().map_err(codec_error)?;
        if input_schema != MOVIE_INPUT_SCHEMA_VERSION {
            return Err(MovieError::UnsupportedInputSchema {
                found: input_schema,
                supported: MOVIE_INPUT_SCHEMA_VERSION,
            });
        }
        let movie_id = reader.read_u64_le().map_err(codec_error)?;
        if movie_id == 0 {
            return Err(MovieError::InvalidMovieId);
        }
        let name_len = usize::from(reader.read_u16_le().map_err(codec_error)?);
        let name = if name_len == 0 {
            None
        } else {
            if name_len > MAX_NAME_BYTES {
                return Err(MovieError::InvalidName(format!(
                    "movie name is {name_len} bytes, limit is {MAX_NAME_BYTES}"
                )));
            }
            let name_bytes = reader.read_exact(name_len).map_err(codec_error)?;
            let name = std::str::from_utf8(name_bytes).map_err(|error| {
                MovieError::InvalidName(format!("movie name is not UTF-8: {error}"))
            })?;
            validate_name(name)?;
            Some(name.to_owned())
        };
        let parent_raw = reader.read_u64_le().map_err(codec_error)?;
        let parent_movie_id = if parent_raw == 0 {
            None
        } else {
            Some(parent_raw)
        };
        if parent_movie_id == Some(movie_id) {
            return Err(MovieError::InvalidParentMovieId);
        }
        let start_tick = Tick::from_raw(reader.read_u64_le().map_err(codec_error)?);
        let snapshot_len_u64 = reader.read_u64_le().map_err(codec_error)?;
        let snapshot_len =
            usize::try_from(snapshot_len_u64).map_err(|_| MovieError::FileTooLarge {
                limit: MAX_MOVIE_BYTES,
                found: snapshot_len_u64,
            })?;
        // The step count and records are still ahead; check that the declared
        // checkpoint length plus the step-count field fit in the remaining body
        // before slicing or copying anything.
        let required = snapshot_len.checked_add(8).ok_or(MovieError::Truncated)?;
        if reader.remaining() < required {
            return Err(MovieError::Truncated);
        }
        let snapshot_bytes = reader.read_exact(snapshot_len).map_err(codec_error)?;
        let snapshot =
            Snapshot::from_bytes(snapshot_bytes).map_err(MovieError::InvalidEmbeddedSnapshot)?;
        if start_tick != snapshot.tick() {
            return Err(MovieError::StartTickMismatch {
                declared: start_tick,
                snapshot: snapshot.tick(),
            });
        }
        let step_count_u64 = reader.read_u64_le().map_err(codec_error)?;
        if step_count_u64 > MAX_STEPS as u64 {
            return Err(MovieError::TooManySteps {
                limit: MAX_STEPS,
                found: step_count_u64,
            });
        }
        let step_count = usize::try_from(step_count_u64).map_err(|_| MovieError::TooManySteps {
            limit: MAX_STEPS,
            found: step_count_u64,
        })?;
        // Bounded by `MAX_STEPS × 44`; overflow is reported instead of wrapping.
        let record_bytes = step_count
            .checked_mul(STEP_RECORD_LEN)
            .ok_or(MovieError::Truncated)?;
        if reader.remaining() < record_bytes {
            return Err(MovieError::Truncated);
        }
        let mut steps = Vec::with_capacity(step_count);
        for _ in 0..step_count {
            let outcome_tick = Tick::from_raw(reader.read_u64_le().map_err(codec_error)?);
            let input = InputFrame::new(reader.read_u32_le().map_err(codec_error)?);
            let state_hash = StateHash::from_bytes(reader.read_array::<32>().map_err(codec_error)?);
            steps.push(MovieStep {
                outcome_tick,
                input,
                state_hash,
            });
        }
        reader.finish().map_err(codec_error)?;
        validate_steps(&steps, start_tick)?;
        Ok(Self {
            meta: MovieMeta {
                movie_id,
                name,
                parent_movie_id,
            },
            snapshot_bytes: snapshot_bytes.to_vec(),
            start_tick,
            start_state_hash: snapshot.state_hash(),
            steps,
        })
    }

    /// Returns the non-canonical movie metadata.
    #[must_use]
    pub const fn meta(&self) -> &MovieMeta {
        &self.meta
    }

    /// Returns the tick of the embedded checkpoint.
    #[must_use]
    pub const fn start_tick(&self) -> Tick {
        self.start_tick
    }

    /// Returns the state digest recorded by the embedded checkpoint.
    ///
    /// This is the state every replay starts from; it is decoded once by
    /// [`Movie::from_parts`]/[`Movie::from_bytes`] and exposed read-only so
    /// ranged replay and comparison never decode the embedded checkpoint again.
    #[must_use]
    pub const fn checkpoint_state_hash(&self) -> StateHash {
        self.start_state_hash
    }

    /// Returns the last recorded outcome tick, or the start tick for a zero-step
    /// movie.
    #[must_use]
    pub fn end_tick(&self) -> Tick {
        self.steps
            .last()
            .map_or(self.start_tick, MovieStep::outcome_tick)
    }

    /// Returns the number of recorded steps.
    #[must_use]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Returns the recorded final state digest, or the embedded checkpoint digest
    /// for a zero-step movie.
    #[must_use]
    pub fn final_state_hash(&self) -> StateHash {
        self.steps
            .last()
            .map_or(self.start_state_hash, MovieStep::state_hash)
    }

    /// Returns the verbatim canonical checkpoint bytes embedded in this movie.
    #[must_use]
    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot_bytes
    }

    /// Returns the ordered step records.
    #[must_use]
    pub fn steps(&self) -> &[MovieStep] {
        &self.steps
    }
}

/// Result of a clean replay verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayReport {
    /// Tick of the embedded checkpoint.
    pub start_tick: Tick,
    /// Last recorded outcome tick (`start_tick` for a zero-step movie).
    pub end_tick: Tick,
    /// Number of replayed and verified steps.
    pub steps: usize,
    /// Final tick observed during replay.
    pub final_tick: Tick,
    /// Final state digest observed during replay.
    pub final_state_hash: StateHash,
}

/// A typed movie format, recording or replay failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MovieError {
    /// The byte slice is shorter than the fixed container overhead.
    TooShort {
        /// Minimum number of bytes.
        minimum: usize,
        /// Observed length.
        found: usize,
    },
    /// The byte slice, a declared length or a declared count exceeds the v1 bound.
    FileTooLarge {
        /// Configured maximum in bytes.
        limit: usize,
        /// Declared or observed size.
        found: u64,
    },
    /// A declared length or count exceeds the bytes that actually remain.
    Truncated,
    /// Bytes remain after the last expected step record.
    TrailingBytes {
        /// Number of unexpected bytes.
        count: usize,
    },
    /// The trailing SHA-256 does not cover the body.
    ChecksumMismatch,
    /// The container magic is not `CFMOVIE1`.
    BadMagic,
    /// The movie format version is not supported by this build.
    UnsupportedFormatVersion {
        /// Version found in the file.
        found: u16,
        /// Version supported by this build.
        supported: u16,
    },
    /// The input schema version is not supported by this build.
    UnsupportedInputSchema {
        /// Version found in the file.
        found: u16,
        /// Version supported by this build.
        supported: u16,
    },
    /// The movie id is zero.
    InvalidMovieId,
    /// The parent movie id is zero or equals the movie id.
    InvalidParentMovieId,
    /// The name is empty, over-long, invalid UTF-8 or contains control characters.
    InvalidName(String),
    /// The declared start tick disagrees with the embedded checkpoint.
    StartTickMismatch {
        /// Tick declared by the movie.
        declared: Tick,
        /// Tick recorded by the embedded checkpoint.
        snapshot: Tick,
    },
    /// The declared step count exceeds [`MAX_STEPS`].
    TooManySteps {
        /// Configured maximum number of steps.
        limit: usize,
        /// Declared or observed count.
        found: u64,
    },
    /// Outcome ticks are not strictly increasing after the start tick.
    TickSequence {
        /// Zero-based step index where the sequence broke.
        input_index: usize,
        /// Previous accepted tick.
        previous: Tick,
        /// Tick found in the file.
        found: Tick,
    },
    /// The requested replay start is before the embedded checkpoint tick.
    RangeBeforeStart {
        /// Requested start boundary.
        from: Tick,
        /// Tick of the embedded checkpoint.
        start_tick: Tick,
    },
    /// The requested replay end is after the movie's last recorded outcome tick.
    RangeAfterEnd {
        /// Requested end boundary.
        until: Tick,
        /// Last recorded outcome tick (`start_tick` for a zero-step movie).
        end_tick: Tick,
    },
    /// The requested replay start is after the requested end.
    RangeInverted {
        /// Requested start boundary.
        from: Tick,
        /// Requested end boundary.
        until: Tick,
    },
    /// The requested replay start is not the start tick or a recorded outcome tick.
    RangeFromUnaligned {
        /// Boundary that is not aligned with the decoded movie.
        tick: Tick,
    },
    /// The requested replay end is not the start tick, end tick or a recorded
    /// outcome tick.
    RangeUntilUnaligned {
        /// Boundary that is not aligned with the decoded movie.
        tick: Tick,
    },
    /// The requested replay window contains no recorded step.
    RangeEmpty {
        /// Requested (equal) start boundary.
        from: Tick,
        /// Requested (equal) end boundary.
        until: Tick,
    },
    /// The embedded checkpoint is malformed, truncated or has a bad checksum.
    InvalidEmbeddedSnapshot(SessionError),
    /// Capturing or serializing the recording checkpoint failed.
    SnapshotCapture(SessionError),
    /// Importing the embedded checkpoint failed, including backend/configuration
    /// incompatibility decided by the existing compatibility predicate.
    SnapshotImport(SessionError),
    /// A backend or session step failed while recording.
    RecordFailed {
        /// Zero-based input index that failed.
        input_index: usize,
        /// Preserved session error.
        source: SessionError,
    },
    /// A backend or session step failed while replaying.
    ReplayFailed {
        /// Zero-based input index that failed.
        input_index: usize,
        /// Preserved session error.
        source: SessionError,
    },
    /// The observed outcome tick differs from the recorded expectation.
    TickMismatch {
        /// Zero-based input index.
        input_index: usize,
        /// Recorded outcome tick.
        expected: Tick,
        /// Observed outcome tick.
        observed: Tick,
    },
    /// The first state-hash divergence, reported at its exact recorded tick.
    Divergence {
        /// Recorded outcome tick of the first mismatching step.
        tick: Tick,
        /// Zero-based step index whose input was applied.
        input_index: usize,
        /// Recorded expected state digest.
        expected: StateHash,
        /// State digest the backend actually produced.
        observed: StateHash,
    },
}

impl fmt::Display for MovieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { minimum, found } => {
                write!(f, "movie is {found} bytes, minimum is {minimum}")
            }
            Self::FileTooLarge { limit, found } => {
                write!(f, "movie is {found} bytes, limit is {limit}")
            }
            Self::Truncated => f.write_str(
                "movie is truncated: a declared length or count exceeds the remaining bytes",
            ),
            Self::TrailingBytes { count } => {
                write!(f, "movie has {count} trailing byte(s)")
            }
            Self::ChecksumMismatch => f.write_str("movie checksum mismatch"),
            Self::BadMagic => f.write_str("movie magic mismatch"),
            Self::UnsupportedFormatVersion { found, supported } => write!(
                f,
                "unsupported movie format version {found}, expected {supported}"
            ),
            Self::UnsupportedInputSchema { found, supported } => write!(
                f,
                "unsupported movie input schema version {found}, expected {supported}"
            ),
            Self::InvalidMovieId => f.write_str("movie id must be non-zero"),
            Self::InvalidParentMovieId => {
                f.write_str("parent movie id must be non-zero and differ from the movie id")
            }
            Self::InvalidName(message) => write!(f, "invalid movie name: {message}"),
            Self::StartTickMismatch { declared, snapshot } => write!(
                f,
                "movie start tick {declared} does not match the embedded checkpoint tick {snapshot}"
            ),
            Self::TooManySteps { limit, found } => {
                write!(f, "movie declares {found} steps, limit is {limit}")
            }
            Self::TickSequence {
                input_index,
                previous,
                found,
            } => write!(
                f,
                "movie tick sequence breaks at input index {input_index}: \
                 tick {found} does not follow {previous}"
            ),
            Self::RangeBeforeStart { from, start_tick } => write!(
                f,
                "replay range start {from} is before the embedded checkpoint tick {start_tick}"
            ),
            Self::RangeAfterEnd { until, end_tick } => write!(
                f,
                "replay range end {until} is after the movie end tick {end_tick}"
            ),
            Self::RangeInverted { from, until } => write!(
                f,
                "replay range is inverted: start {from} is after end {until}"
            ),
            Self::RangeFromUnaligned { tick } => write!(
                f,
                "replay range start tick {tick} is not the movie start tick or a recorded outcome tick"
            ),
            Self::RangeUntilUnaligned { tick } => write!(
                f,
                "replay range end tick {tick} is not the movie start tick, end tick or a recorded outcome tick"
            ),
            Self::RangeEmpty { from, until } => {
                write!(f, "replay range {from}..={until} contains no recorded step")
            }
            Self::InvalidEmbeddedSnapshot(error) => {
                write!(f, "invalid embedded checkpoint: {error}")
            }
            Self::SnapshotCapture(error) => {
                write!(f, "cannot capture the recording checkpoint: {error}")
            }
            Self::SnapshotImport(error) => {
                write!(f, "cannot import the embedded checkpoint: {error}")
            }
            Self::RecordFailed {
                input_index,
                source,
            } => write!(f, "recording failed at input index {input_index}: {source}"),
            Self::ReplayFailed {
                input_index,
                source,
            } => write!(f, "replay failed at input index {input_index}: {source}"),
            Self::TickMismatch {
                input_index,
                expected,
                observed,
            } => write!(
                f,
                "recorded tick mismatch at input index {input_index}: \
                 expected {expected}, observed {observed}"
            ),
            Self::Divergence {
                tick,
                input_index,
                expected,
                observed,
            } => write!(
                f,
                "state divergence at tick {tick} (input index {input_index}): \
                 expected {expected}, observed {observed}"
            ),
        }
    }
}

impl Error for MovieError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidEmbeddedSnapshot(error)
            | Self::SnapshotCapture(error)
            | Self::SnapshotImport(error)
            | Self::RecordFailed { source: error, .. }
            | Self::ReplayFailed { source: error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Validates the optional name against the single v1 name rule.
fn validate_name(name: &str) -> Result<(), MovieError> {
    if name.is_empty() {
        return Err(MovieError::InvalidName(
            "movie name must not be empty".to_owned(),
        ));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(MovieError::InvalidName(format!(
            "movie name is {} bytes, limit is {MAX_NAME_BYTES}",
            name.len()
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(MovieError::InvalidName(
            "movie name must not contain control characters".to_owned(),
        ));
    }
    Ok(())
}

/// Validates that every recorded outcome tick strictly follows the previous one.
fn validate_steps(steps: &[MovieStep], start_tick: Tick) -> Result<(), MovieError> {
    let mut previous = start_tick;
    for (input_index, step) in steps.iter().enumerate() {
        if step.outcome_tick <= previous {
            return Err(MovieError::TickSequence {
                input_index,
                previous,
                found: step.outcome_tick,
            });
        }
        previous = step.outcome_tick;
    }
    Ok(())
}

/// Rejects a byte slice outside the v1 container bounds before it is parsed.
fn validate_slice_len(len: usize) -> Result<(), MovieError> {
    if len < FIXED_OVERHEAD {
        return Err(MovieError::TooShort {
            minimum: FIXED_OVERHEAD,
            found: len,
        });
    }
    if len > MAX_MOVIE_BYTES {
        return Err(MovieError::FileTooLarge {
            limit: MAX_MOVIE_BYTES,
            found: len as u64,
        });
    }
    Ok(())
}

/// Computes the exact encoded length after checking the v1 count and size limits.
fn checked_movie_len(
    name_len: usize,
    snapshot_len: usize,
    step_count: usize,
) -> Result<usize, MovieError> {
    if name_len > MAX_NAME_BYTES {
        return Err(MovieError::InvalidName(format!(
            "movie name is {name_len} bytes, limit is {MAX_NAME_BYTES}"
        )));
    }
    if step_count > MAX_STEPS {
        return Err(MovieError::TooManySteps {
            limit: MAX_STEPS,
            found: step_count as u64,
        });
    }
    let overflow = MovieError::FileTooLarge {
        limit: MAX_MOVIE_BYTES,
        found: u64::MAX,
    };
    let step_bytes = step_count
        .checked_mul(STEP_RECORD_LEN)
        .ok_or(overflow.clone())?;
    let total = FIXED_OVERHEAD
        .checked_add(name_len)
        .and_then(|value| value.checked_add(snapshot_len))
        .and_then(|value| value.checked_add(step_bytes))
        .ok_or(overflow)?;
    if total > MAX_MOVIE_BYTES {
        return Err(MovieError::FileTooLarge {
            limit: MAX_MOVIE_BYTES,
            found: total as u64,
        });
    }
    Ok(total)
}

/// Translates a shared-codec failure into the movie error contract.
fn codec_error(error: CodecError) -> MovieError {
    match error {
        CodecError::Truncated { .. } => MovieError::Truncated,
        CodecError::LengthOverflow => MovieError::FileTooLarge {
            limit: MAX_MOVIE_BYTES,
            found: u64::MAX,
        },
        CodecError::TrailingBytes { count } => MovieError::TrailingBytes { count },
        CodecError::InvalidUtf8(error) => {
            MovieError::InvalidName(format!("movie name is not UTF-8: {error}"))
        }
        CodecError::EmptyString => {
            MovieError::InvalidName("movie name must not be empty".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::BackendFactory;
    use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick, hash_bytes};
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{
        CHECKSUM_LEN, FIXED_OVERHEAD, MAX_MOVIE_BYTES, MAX_NAME_BYTES, MAX_STEPS,
        MOVIE_FORMAT_VERSION, MOVIE_INPUT_SCHEMA_VERSION, Movie, MovieError, MovieMeta, MovieStep,
        STEP_RECORD_LEN, checked_movie_len, validate_slice_len,
    };
    use crate::SessionError;
    use crate::snapshot::Snapshot;

    /// Byte offset of the `name_length` field in the canonical layout.
    const NAME_LENGTH_OFFSET: usize = 20;
    /// Byte offset of the optional name body.
    const NAME_OFFSET: usize = 22;

    fn checkpoint() -> Snapshot {
        let backend = SyntheticBackendFactory::default()
            .create()
            .expect("synthetic backend");
        let backend = backend.snapshot().expect("synthetic checkpoint");
        Snapshot::new(SnapshotId::from_raw(9), SessionId::from_raw(4), backend)
    }

    fn checkpoint_bytes() -> Vec<u8> {
        checkpoint().to_bytes().expect("checkpoint bytes")
    }

    fn meta() -> MovieMeta {
        MovieMeta::new(11, Some("branch-a".to_owned()), Some(3)).expect("movie metadata")
    }

    fn steps() -> Vec<MovieStep> {
        (1..=4_u8)
            .map(|index| {
                MovieStep::new(
                    Tick::from_raw(u64::from(index)),
                    InputFrame::new(u32::from(index)),
                    StateHash::from_bytes([index; 32]),
                )
            })
            .collect()
    }

    fn movie() -> Movie {
        Movie::from_parts(meta(), checkpoint_bytes(), steps()).expect("sample movie")
    }

    /// Recomputes the trailing checksum after a deliberate structural mutation.
    fn resign(bytes: &mut [u8]) {
        let body_len = bytes.len() - CHECKSUM_LEN;
        let checksum = hash_bytes(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(checksum.as_bytes());
    }

    fn resigned(mut bytes: Vec<u8>) -> Vec<u8> {
        resign(&mut bytes);
        bytes
    }

    /// Mutates a field and rebuilds the checksum so a later validator is exercised.
    fn mutate(movie: &Movie, offset: usize, value: &[u8]) -> Vec<u8> {
        let mut bytes = movie.to_bytes().expect("encode movie");
        bytes[offset..offset + value.len()].copy_from_slice(value);
        resign(&mut bytes);
        bytes
    }

    fn name_len(movie: &Movie) -> usize {
        movie.meta().name().map_or(0, str::len)
    }

    fn parent_offset(movie: &Movie) -> usize {
        NAME_OFFSET + name_len(movie)
    }

    fn start_tick_offset(movie: &Movie) -> usize {
        parent_offset(movie) + 8
    }

    fn snapshot_len_offset(movie: &Movie) -> usize {
        start_tick_offset(movie) + 8
    }

    fn step_count_offset(movie: &Movie) -> usize {
        snapshot_len_offset(movie) + 8 + movie.snapshot_bytes().len()
    }

    fn record_offset(movie: &Movie) -> usize {
        step_count_offset(movie) + 8
    }

    #[test]
    fn round_trip_preserves_metadata_checkpoint_and_steps() {
        let original = movie();
        let bytes = original.to_bytes().expect("encode movie");
        let decoded = Movie::from_bytes(&bytes).expect("decode movie");
        assert_eq!(decoded, original);
        assert_eq!(decoded.meta().movie_id(), 11);
        assert_eq!(decoded.meta().name(), Some("branch-a"));
        assert_eq!(decoded.meta().parent_movie_id(), Some(3));
        assert_eq!(decoded.start_tick(), Tick::ZERO);
        assert_eq!(decoded.end_tick(), Tick::from_raw(4));
        assert_eq!(decoded.step_count(), 4);
        assert_eq!(decoded.final_state_hash(), StateHash::from_bytes([4; 32]));
        assert_eq!(decoded.to_bytes().expect("re-encode"), bytes);
    }

    #[test]
    fn embedded_checkpoint_is_exactly_snapshot_to_bytes() {
        let snapshot = checkpoint();
        let expected = snapshot.to_bytes().expect("snapshot bytes");
        let movie = Movie::from_parts(meta(), expected.clone(), steps())
            .expect("movie from snapshot bytes");
        assert_eq!(movie.snapshot_bytes(), expected.as_slice());
        assert_eq!(
            Snapshot::from_bytes(movie.snapshot_bytes()).expect("decode embedded checkpoint"),
            snapshot
        );
    }

    #[test]
    fn encoding_is_byte_deterministic_across_independent_runs() {
        // Two independent constructions and encodings. The encoder reads no clock,
        // path, process state or entropy, so a later run in another process is
        // byte-equal as well.
        let first = Movie::from_parts(meta(), checkpoint_bytes(), steps())
            .expect("first movie")
            .to_bytes()
            .expect("first encoding");
        std::thread::yield_now();
        let second = Movie::from_parts(meta(), checkpoint_bytes(), steps())
            .expect("second movie")
            .to_bytes()
            .expect("second encoding");
        assert_eq!(first, second);
        assert_eq!(hash_bytes(&first), hash_bytes(&second));
        let decoded = Movie::from_bytes(&second).expect("decode second encoding");
        assert_eq!(decoded.to_bytes().expect("re-encode decoded movie"), second);
    }

    #[test]
    fn zero_step_movie_is_valid_and_ends_at_the_checkpoint() {
        let snapshot = checkpoint();
        let movie = Movie::from_parts(meta(), snapshot.to_bytes().expect("snapshot"), Vec::new())
            .expect("zero-step movie");
        assert_eq!(movie.step_count(), 0);
        assert_eq!(movie.start_tick(), snapshot.tick());
        assert_eq!(movie.end_tick(), snapshot.tick());
        assert_eq!(movie.final_state_hash(), snapshot.state_hash());
        let decoded = Movie::from_bytes(&movie.to_bytes().expect("encode")).expect("decode");
        assert_eq!(decoded, movie);
    }

    #[test]
    fn absent_name_and_parent_encode_as_sentinels() {
        let movie = Movie::from_parts(
            MovieMeta::new(1, None, None).expect("metadata"),
            checkpoint_bytes(),
            steps(),
        )
        .expect("movie");
        let bytes = movie.to_bytes().expect("encode");
        assert_eq!(
            &bytes[NAME_LENGTH_OFFSET..NAME_LENGTH_OFFSET + 2],
            &0_u16.to_le_bytes()
        );
        let decoded = Movie::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded.meta().name(), None);
        assert_eq!(decoded.meta().parent_movie_id(), None);
    }

    #[test]
    fn checksum_covers_every_preceding_byte() {
        let bytes = movie().to_bytes().expect("encode");
        for (index, byte) in bytes.iter().enumerate() {
            let mut corrupted = bytes.clone();
            corrupted[index] = byte ^ 0xff;
            assert!(
                Movie::from_bytes(&corrupted).is_err(),
                "flipping byte {index} must be rejected"
            );
        }
    }

    #[test]
    fn magic_and_version_rejections_are_typed() {
        let movie = movie();
        let bad_magic = mutate(&movie, 0, b"X");
        assert!(matches!(
            Movie::from_bytes(&bad_magic),
            Err(MovieError::BadMagic)
        ));

        let bad_format = mutate(&movie, 8, &2_u16.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&bad_format),
            Err(MovieError::UnsupportedFormatVersion { found: 2, supported })
                if supported == MOVIE_FORMAT_VERSION
        ));

        let bad_schema = mutate(&movie, 10, &0_u16.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&bad_schema),
            Err(MovieError::UnsupportedInputSchema { found: 0, supported })
                if supported == MOVIE_INPUT_SCHEMA_VERSION
        ));

        let mut checksum = movie.to_bytes().expect("encode");
        let last = checksum.len() - 1;
        checksum[last] ^= 0xff;
        assert!(matches!(
            Movie::from_bytes(&checksum),
            Err(MovieError::ChecksumMismatch)
        ));
    }

    #[test]
    fn every_truncation_is_rejected_without_panicking() {
        let bytes = movie().to_bytes().expect("encode");
        for cut in 0..bytes.len() {
            assert!(
                Movie::from_bytes(&bytes[..cut]).is_err(),
                "truncated at {cut} bytes must be rejected"
            );
        }
        assert!(matches!(
            Movie::from_bytes(&[]),
            Err(MovieError::TooShort {
                minimum: FIXED_OVERHEAD,
                found: 0
            })
        ));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let bytes = movie().to_bytes().expect("encode");
        let mut appended = bytes.clone();
        appended.push(0);
        assert!(matches!(
            Movie::from_bytes(&appended),
            Err(MovieError::ChecksumMismatch)
        ));

        let mut body = bytes[..bytes.len() - CHECKSUM_LEN].to_vec();
        body.push(0);
        body.extend_from_slice(&[0; CHECKSUM_LEN]);
        assert!(matches!(
            Movie::from_bytes(&resigned(body)),
            Err(MovieError::TrailingBytes { count: 1 })
        ));
    }

    #[test]
    fn declared_lengths_and_counts_are_bounded_before_allocation() {
        let movie = movie();
        let huge_snapshot = mutate(&movie, snapshot_len_offset(&movie), &u64::MAX.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&huge_snapshot),
            Err(MovieError::Truncated)
        ));

        let huge_steps = mutate(&movie, step_count_offset(&movie), &u64::MAX.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&huge_steps),
            Err(MovieError::TooManySteps {
                limit: MAX_STEPS,
                found: u64::MAX
            })
        ));

        let one_step = Movie::from_parts(meta(), checkpoint_bytes(), steps()[..1].to_vec())
            .expect("one-step movie");
        let over_remaining = mutate(
            &one_step,
            step_count_offset(&one_step),
            &2_u64.to_le_bytes(),
        );
        assert!(matches!(
            Movie::from_bytes(&over_remaining),
            Err(MovieError::Truncated)
        ));
    }

    #[test]
    fn container_size_bounds_are_enforced_before_parsing() {
        assert!(matches!(
            validate_slice_len(MAX_MOVIE_BYTES + 1),
            Err(MovieError::FileTooLarge {
                limit: MAX_MOVIE_BYTES,
                ..
            })
        ));
        assert!(matches!(
            validate_slice_len(FIXED_OVERHEAD - 1),
            Err(MovieError::TooShort {
                minimum: FIXED_OVERHEAD,
                ..
            })
        ));
        assert_eq!(validate_slice_len(FIXED_OVERHEAD), Ok(()));
        assert_eq!(validate_slice_len(MAX_MOVIE_BYTES), Ok(()));

        assert!(matches!(
            checked_movie_len(0, 0, MAX_STEPS + 1),
            Err(MovieError::TooManySteps { .. })
        ));
        assert!(matches!(
            checked_movie_len(MAX_NAME_BYTES + 1, 0, 0),
            Err(MovieError::InvalidName(_))
        ));
        assert!(matches!(
            checked_movie_len(0, MAX_MOVIE_BYTES, 0),
            Err(MovieError::FileTooLarge { .. })
        ));
        assert!(matches!(
            checked_movie_len(0, usize::MAX, 0),
            Err(MovieError::FileTooLarge { .. })
        ));
        assert_eq!(checked_movie_len(0, 0, 0), Ok(FIXED_OVERHEAD));
    }

    #[test]
    fn metadata_validation_is_typed() {
        assert!(matches!(
            MovieMeta::new(0, None, None),
            Err(MovieError::InvalidMovieId)
        ));
        assert!(matches!(
            MovieMeta::new(1, None, Some(0)),
            Err(MovieError::InvalidParentMovieId)
        ));
        assert!(matches!(
            MovieMeta::new(1, None, Some(1)),
            Err(MovieError::InvalidParentMovieId)
        ));
        assert!(matches!(
            MovieMeta::new(1, Some(String::new()), None),
            Err(MovieError::InvalidName(_))
        ));
        assert!(matches!(
            MovieMeta::new(1, Some("a".repeat(MAX_NAME_BYTES + 1)), None),
            Err(MovieError::InvalidName(_))
        ));
        assert!(matches!(
            MovieMeta::new(1, Some("bad\nname".to_owned()), None),
            Err(MovieError::InvalidName(_))
        ));
        assert!(MovieMeta::new(1, Some("a".repeat(MAX_NAME_BYTES)), None).is_ok());

        let movie = movie();
        let zero_id = mutate(&movie, 12, &0_u64.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&zero_id),
            Err(MovieError::InvalidMovieId)
        ));

        let self_parent = mutate(
            &movie,
            parent_offset(&movie),
            &movie.meta().movie_id().to_le_bytes(),
        );
        assert!(matches!(
            Movie::from_bytes(&self_parent),
            Err(MovieError::InvalidParentMovieId)
        ));

        let too_long = u16::try_from(MAX_NAME_BYTES).expect("name bound fits u16") + 1;
        let long_name = mutate(&movie, NAME_LENGTH_OFFSET, &too_long.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&long_name),
            Err(MovieError::InvalidName(_))
        ));

        let invalid_utf8 = mutate(&movie, NAME_OFFSET, &[0xff]);
        assert!(matches!(
            Movie::from_bytes(&invalid_utf8),
            Err(MovieError::InvalidName(_))
        ));

        let control = mutate(&movie, NAME_OFFSET, &[0x01]);
        assert!(matches!(
            Movie::from_bytes(&control),
            Err(MovieError::InvalidName(_))
        ));
    }

    #[test]
    fn start_tick_must_match_the_embedded_checkpoint() {
        let movie = movie();
        let altered = mutate(
            &movie,
            start_tick_offset(&movie),
            &(movie.start_tick().raw() + 1).to_le_bytes(),
        );
        assert!(matches!(
            Movie::from_bytes(&altered),
            Err(MovieError::StartTickMismatch { declared, snapshot })
                if declared == Tick::from_raw(1) && snapshot == Tick::ZERO
        ));
    }

    #[test]
    fn tick_sequence_is_validated_from_the_start_tick() {
        let movie = movie();
        let second_tick = record_offset(&movie) + STEP_RECORD_LEN;
        let repeated = mutate(&movie, second_tick, &1_u64.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&repeated),
            Err(MovieError::TickSequence {
                input_index: 1,
                previous,
                found
            }) if previous == Tick::from_raw(1) && found == Tick::from_raw(1)
        ));

        let first_tick = record_offset(&movie);
        let before_start = mutate(&movie, first_tick, &0_u64.to_le_bytes());
        assert!(matches!(
            Movie::from_bytes(&before_start),
            Err(MovieError::TickSequence {
                input_index: 0,
                previous,
                found
            }) if previous == Tick::ZERO && found == Tick::ZERO
        ));
    }

    #[test]
    fn embedded_checkpoint_rejections_are_typed_and_preserve_the_session_error() {
        let empty = Movie::from_parts(meta(), Vec::new(), steps());
        assert!(matches!(
            empty,
            Err(MovieError::InvalidEmbeddedSnapshot(
                SessionError::InvalidSnapshot(_)
            ))
        ));

        let mut truncated = checkpoint_bytes();
        truncated.pop();
        assert!(matches!(
            Movie::from_parts(meta(), truncated, steps()),
            Err(MovieError::InvalidEmbeddedSnapshot(
                SessionError::InvalidSnapshot(_)
            ))
        ));

        let mut corrupt = checkpoint_bytes();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        assert!(matches!(
            Movie::from_parts(meta(), corrupt, steps()),
            Err(MovieError::InvalidEmbeddedSnapshot(
                SessionError::InvalidSnapshot(_)
            ))
        ));

        let mut incompatible = checkpoint_bytes();
        incompatible[8..10].copy_from_slice(&2_u16.to_le_bytes());
        resign(&mut incompatible);
        assert!(matches!(
            Movie::from_parts(meta(), incompatible, steps()),
            Err(MovieError::InvalidEmbeddedSnapshot(
                SessionError::IncompatibleSnapshot(_)
            ))
        ));
    }

    #[test]
    fn arbitrary_bytes_are_rejected_without_panicking() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for length in 0..=2048 {
            let mut bytes = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push((state >> 24).to_le_bytes()[0]);
            }
            assert!(
                Movie::from_bytes(&bytes).is_err(),
                "arbitrary {length}-byte movie must be rejected"
            );
        }
    }

    #[test]
    fn metadata_changes_do_not_change_recorded_execution() {
        let base = Movie::from_parts(meta(), checkpoint_bytes(), steps()).expect("base movie");
        let renamed = Movie::from_parts(
            MovieMeta::new(11, Some("other-name".to_owned()), Some(3)).expect("renamed metadata"),
            checkpoint_bytes(),
            steps(),
        )
        .expect("renamed movie");
        let reparented = Movie::from_parts(
            MovieMeta::new(11, Some("branch-a".to_owned()), Some(99)).expect("reparented metadata"),
            checkpoint_bytes(),
            steps(),
        )
        .expect("reparented movie");

        let base_bytes = base.to_bytes().expect("base bytes");
        assert_ne!(base_bytes, renamed.to_bytes().expect("renamed bytes"));
        assert_ne!(base_bytes, reparented.to_bytes().expect("reparented bytes"));
        for other in [&renamed, &reparented] {
            assert_eq!(other.start_tick(), base.start_tick());
            assert_eq!(other.end_tick(), base.end_tick());
            assert_eq!(other.step_count(), base.step_count());
            assert_eq!(other.final_state_hash(), base.final_state_hash());
            assert_eq!(other.steps(), base.steps());
        }
    }
}
