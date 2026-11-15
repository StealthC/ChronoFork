//! Canonical `.cfmovie` container, recorder and verifier.
//!
//! One reader ([`Movie::from_bytes`]) and one writer ([`Movie::to_bytes`]) handle
//! the accepted v1 format of
//! [ADR-0004](../../../../docs/adr/0004-tas-movie-format.md) and the v2 format of
//! [ADR-0006](../../../../docs/adr/0006-movie-format-v2-anchors-and-multi-device-input.md)
//! as a version branch of one codec, built on [`chronofork_core::codec`]. The
//! embedded checkpoint and every anchor snapshot are encoded only by
//! [`Snapshot::to_bytes`] and decoded only by [`Snapshot::from_bytes`]; backend and
//! configuration compatibility are decided only by the existing import path
//! [`SessionManager::create_session_from_snapshot_bytes`], never here.
//!
//! v1 is a self-contained file that embeds the exact canonical checkpoint bytes,
//! the ordered backend-defined input sequence and a per-step expectation record,
//! followed by one trailing SHA-256. The v1 byte layout, limits, decode order,
//! typed errors and encoded bytes are unchanged by the v2 addition.
//!
//! v2 (`CFMOVIE2`) adds an ordered table of checkpoint anchors, each a verbatim
//! canonical [`Snapshot::to_bytes`] envelope bound to a recorded step, and 72-byte
//! schema-2 step records carrying a multi-device [`InputFrame`]. Its limits and
//! typed rejection categories are those of ADR-0006. The v2 branch lives in the
//! internal `v2` module of this file; it is a version branch of this module, not a second
//! reader, writer, checksum rule, snapshot parser or compatibility predicate.
//!
//! [`Movie::from_parts`] and [`Movie::record`] remain the v1 construction and
//! recording entry points; [`Movie::from_v2_parts`] and [`Movie::record_v2`] are
//! the explicit v2 entry points. There is no automatic upgrade or downgrade.
//!
//! Recording captures the source checkpoint and executes the inputs in a separate
//! imported session, so the source tick and state are never advanced. Replay
//! imports a checkpoint into a fresh session and stops at the first mismatch,
//! reporting its exact recorded tick, input index, expected hash and observed
//! hash.
//!
//! Ranged replay over a validated tick window and comparison of two movies'
//! recorded expectations are owned by [`crate::replay`]; [`Movie::verify`] is the
//! full range of that same replay path, so the checkpoint import and the
//! stepping/verification loop exist exactly once. Anchor-based seek is a later
//! replay stage and is not implemented here: this module only exposes the
//! read-only anchor accessors it needs.
//!
//! Deferred (not implemented and not claimed): anchor-based seek execution, an
//! in-process rewind/state cache, interactive stepping, rerecord, branch editing,
//! checkpoint-reference movies, import/export adapters and real-hardware input
//! TAS.

use std::error::Error;
use std::fmt;

use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{InputFrame, SessionId, StateHash, Tick, hash_bytes};

use crate::error::SessionError;
use crate::manager::SessionManager;
use crate::replay::ReplayRange;
use crate::snapshot::Snapshot;

/// Version of the canonical v1 movie container.
pub const MOVIE_FORMAT_VERSION: u16 = 1;

/// Version of the v1 recorded input schema: one backend-defined `u32` mask per step.
pub const MOVIE_INPUT_SCHEMA_VERSION: u16 = 1;

/// Version of the canonical v2 movie container (`CFMOVIE2`).
pub const V2_MOVIE_FORMAT_VERSION: u16 = 2;

/// Version of the v2 recorded input schema: one multi-device frame per step.
pub const V2_MOVIE_INPUT_SCHEMA_VERSION: u16 = 2;

/// Maximum size of one complete movie file, including the trailing checksum.
///
/// A movie larger than this is rejected by the reader before any body is parsed,
/// and the writer refuses to produce one.
pub const MAX_MOVIE_BYTES: usize = 256 * 1024 * 1024;

/// Maximum number of recorded steps in one v1 movie.
///
/// The bound and [`MAX_MOVIE_BYTES`] are consistent: `MAX_STEPS × 44` bytes is
/// 184,549,376 bytes, leaving the remaining file budget for metadata and the
/// embedded checkpoint.
pub const MAX_STEPS: usize = 4_194_304;

/// Maximum number of recorded steps in one v2 movie.
///
/// A schema-2 step record is 72 bytes rather than 44, so the v2 cap keeps every
/// maximal v2 combination inside [`MAX_MOVIE_BYTES`]. Raising it requires an
/// ADR-0006 amendment and new rejection tests.
pub const MAX_V2_STEPS: usize = 2_000_000;

/// Maximum number of anchor records in one v2 movie.
pub const MAX_ANCHORS: usize = 65_536;

/// Maximum size of the embedded initial checkpoint in a v2 movie.
///
/// This reuses the durable per-checkpoint budget class of ADR-0003.
pub const MAX_EMBEDDED_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Maximum size of one anchor snapshot in a v2 movie.
pub const MAX_ANCHOR_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Maximum cumulative anchor snapshot bytes in one v2 movie.
pub const MAX_ANCHOR_BYTES: usize = 48 * 1024 * 1024;

/// Maximum number of UTF-8 bytes in the optional movie name.
///
/// Both formats fix this value at 256 bytes. It reuses the session-layer name
/// bound so the workspace has one numeric source, but a future change to that
/// constant must not change the movie readers until the format version is bumped
/// with updated rejection tests.
pub const MAX_NAME_BYTES: usize = crate::storage::MAX_NAME_BYTES;

const MAGIC: [u8; 8] = *b"CFMOVIE1";
const V2_MAGIC: [u8; 8] = *b"CFMOVIE2";
const CHECKSUM_LEN: usize = 32;
const STEP_RECORD_LEN: usize = 8 + 4 + 32;
/// One v2 schema-2 step record: tick, four ports, four axes, keyboard, state hash.
pub const V2_STEP_RECORD_LEN: usize = 8 + 4 * 4 + 2 * 4 + 8 + 32;
/// One v2 anchor's framing: the step index and the snapshot length fields.
pub const ANCHOR_RECORD_OVERHEAD: usize = 8 + 8;
/// Fixed v2 container overhead before the optional name, snapshots and records.
///
/// `8 magic + 2 format + 2 schema + 8 movie id + 2 name length + 8 parent id`
/// `+ 8 start tick + 8 snapshot length + 8 anchor count + 8 step count`
/// `+ 32 checksum = 94`.
pub const V2_FIXED_OVERHEAD: usize = V2_MAGIC.len()
    + 2  // movie format version
    + 2  // input schema version
    + 8  // movie id
    + 2  // name length
    + 8  // parent movie id
    + 8  // start tick
    + 8  // embedded snapshot length
    + 8  // anchor count
    + 8  // step count
    + CHECKSUM_LEN;
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
    /// The validating constructors [`Movie::from_parts`] and
    /// [`Movie::from_v2_parts`] check the resulting tick sequence before a movie
    /// can be encoded.
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

    /// Returns the applied input frame.
    ///
    /// A v1 movie reconstructs only its recorded port-0 mask, so the other device
    /// fields are neutral; a v2 movie reconstructs the full schema-2 frame.
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

/// One checkpoint anchor of a v2 movie.
///
/// An anchor is a restorable checkpoint captured after the recorded step at
/// zero-based index [`MovieAnchor::step_index`]. It stores the verbatim canonical
/// [`Snapshot::to_bytes`] bytes of that checkpoint; the tick and state digest are
/// read once by [`Snapshot::from_bytes`] and bound to the recorded step by
/// [`Movie::from_v2_parts`] or the v2 reader. A standalone anchor does not verify
/// that binding: its snapshot has not been placed against a recorded step yet.
///
/// The embedded initial checkpoint is the implicit anchor before step 0 and is
/// never repeated in the anchor table, so a zero-step movie has no valid anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovieAnchor {
    step_index: usize,
    tick: Tick,
    state_hash: StateHash,
    snapshot_bytes: Vec<u8>,
}

impl MovieAnchor {
    /// Validates and creates an anchor for the recorded step at `step_index`.
    ///
    /// The bytes are parsed exactly once by [`Snapshot::from_bytes`]. A malformed,
    /// truncated, checksum-bad or version-incompatible envelope is
    /// [`MovieError::InvalidAnchorSnapshot`] preserving the session error. A
    /// standalone anchor has no table position yet, so that error names ordinal
    /// `0`; [`Movie::from_v2_parts`] reports the real ordinal for an anchor in a
    /// table. Range and step binding are validated there, not here.
    pub fn new(step_index: usize, snapshot_bytes: Vec<u8>) -> Result<Self, MovieError> {
        Self::with_ordinal(0, step_index, snapshot_bytes)
    }

    /// Creates an anchor, reporting `ordinal` if the snapshot envelope is invalid.
    fn with_ordinal(
        ordinal: usize,
        step_index: usize,
        snapshot_bytes: Vec<u8>,
    ) -> Result<Self, MovieError> {
        let snapshot = Snapshot::from_bytes(&snapshot_bytes).map_err(|source| {
            MovieError::InvalidAnchorSnapshot {
                anchor_ordinal: ordinal,
                source,
            }
        })?;
        Ok(Self {
            step_index,
            tick: snapshot.tick(),
            state_hash: snapshot.state_hash(),
            snapshot_bytes,
        })
    }

    /// Returns the zero-based recorded step this anchor was captured after.
    #[must_use]
    pub const fn step_index(&self) -> usize {
        self.step_index
    }

    /// Returns the tick recorded by the anchor checkpoint.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Returns the state digest recorded by the anchor checkpoint.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.state_hash
    }

    /// Returns the verbatim canonical checkpoint bytes of this anchor.
    #[must_use]
    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot_bytes
    }
}

/// A validated v1 or v2 movie.
///
/// Values are only produced by [`Movie::from_bytes`] (decoding),
/// [`Movie::from_parts`]/[`Movie::from_v2_parts`] (validating construction) or
/// [`Movie::record`]/[`Movie::record_v2`] (recording). Fields are private; a
/// divergent movie for a test is built through the matching validating
/// constructor with a deliberately altered record and a freshly computed
/// trailing checksum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Movie {
    meta: MovieMeta,
    snapshot_bytes: Vec<u8>,
    start_tick: Tick,
    start_state_hash: StateHash,
    format_version: u16,
    input_schema_version: u16,
    anchors: Vec<MovieAnchor>,
    steps: Vec<MovieStep>,
}

impl Movie {
    /// Builds a v1 movie from validated metadata, a canonical snapshot and steps.
    ///
    /// The snapshot bytes must be exactly [`Snapshot::to_bytes`] output for a
    /// compatible checkpoint; they are decoded once by [`Snapshot::from_bytes`]
    /// to derive the start tick and state digest. The steps must be strictly
    /// increasing and start after the checkpoint tick, and every step input must
    /// be v1-encodable: a multi-device frame is rejected with
    /// [`MovieError::UnsupportedInputSchema`] naming schema 2 rather than being
    /// silently truncated to its port-0 mask. The value is rejected when it would
    /// exceed [`MAX_STEPS`] or [`MAX_MOVIE_BYTES`]. This is the single v1
    /// validating construction path; it delegates encoding to [`Movie::to_bytes`].
    pub fn from_parts(
        meta: MovieMeta,
        snapshot_bytes: Vec<u8>,
        steps: Vec<MovieStep>,
    ) -> Result<Self, MovieError> {
        let snapshot =
            Snapshot::from_bytes(&snapshot_bytes).map_err(MovieError::InvalidEmbeddedSnapshot)?;
        validate_steps(&steps, snapshot.tick())?;
        reject_v1_device_steps(&steps)?;
        let movie = Self {
            meta,
            snapshot_bytes,
            start_tick: snapshot.tick(),
            start_state_hash: snapshot.state_hash(),
            format_version: MOVIE_FORMAT_VERSION,
            input_schema_version: MOVIE_INPUT_SCHEMA_VERSION,
            anchors: Vec::new(),
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
    /// permitted source-side effect). A multi-device input is a schema-2 request
    /// and is rejected before any step executes; use [`Movie::record_v2`] for it.
    /// A failed or prematurely halted step yields a typed error and no movie.
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
        reject_v1_device_inputs(inputs)?;
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
    /// The writer is selected by the movie's format version: a v1 value encodes
    /// with the unchanged v1 layout, a v2 value with the v2 layout of ADR-0006.
    /// Both are the same public writer; there is no separate v2 serializer. The
    /// output is deterministic: it contains only fixed-width little-endian
    /// fields, the optional name bytes, the verbatim checkpoint and anchor bytes
    /// and the step records. Host time, paths and other host-dependent values
    /// never enter the encoding.
    pub fn to_bytes(&self) -> Result<Vec<u8>, MovieError> {
        match self.format_version {
            MOVIE_FORMAT_VERSION => self.encode_v1(),
            V2_MOVIE_FORMAT_VERSION => v2::encode(self),
            found => Err(MovieError::UnsupportedFormatVersion {
                found,
                supported: MOVIE_FORMAT_VERSION,
            }),
        }
    }

    /// Encodes the unchanged v1 byte layout.
    fn encode_v1(&self) -> Result<Vec<u8>, MovieError> {
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
    /// no backend. It verifies the trailing checksum before trusting any field and
    /// then dispatches on the exact magic: `CFMOVIE1` enters the retained v1
    /// decode path and `CFMOVIE2` the v2 path. Each path validates its own format
    /// and schema version. Declared lengths and counts are validated against the
    /// format's limits and the bytes that actually remain before any buffer is
    /// allocated; trailing bytes are rejected. Arbitrary input never panics.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MovieError> {
        validate_slice_len(bytes.len())?;
        let body_len = bytes.len() - CHECKSUM_LEN;
        let (body, checksum) = bytes.split_at(body_len);
        if hash_bytes(body).as_bytes() != checksum {
            return Err(MovieError::ChecksumMismatch);
        }

        let mut reader = Reader::new(body);
        let magic = reader.read_array::<8>().map_err(codec_error)?;
        if magic == MAGIC {
            Self::decode_v1(&mut reader)
        } else if magic == V2_MAGIC {
            v2::decode(&mut reader, bytes.len())
        } else {
            Err(MovieError::BadMagic)
        }
    }

    /// Decodes and validates the unchanged v1 body after the magic.
    fn decode_v1(reader: &mut Reader<'_>) -> Result<Self, MovieError> {
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
            format_version: MOVIE_FORMAT_VERSION,
            input_schema_version: MOVIE_INPUT_SCHEMA_VERSION,
            anchors: Vec::new(),
            steps,
        })
    }

    /// Returns the non-canonical movie metadata.
    #[must_use]
    pub const fn meta(&self) -> &MovieMeta {
        &self.meta
    }

    /// Returns the movie format version: `1` or `2`.
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// Returns the recorded input schema version: `1` or `2`.
    #[must_use]
    pub const fn input_schema_version(&self) -> u16 {
        self.input_schema_version
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

    /// Returns the ordered anchor table; empty for v1 and anchor-less v2 movies.
    ///
    /// Indices are strictly increasing and bound to the recorded step at the same
    /// index, so this slice is already validated when it is handed out.
    #[must_use]
    pub fn anchors(&self) -> &[MovieAnchor] {
        &self.anchors
    }

    /// Returns the zero-based recorded step whose outcome tick equals `tick`.
    ///
    /// Recorded ticks are strictly increasing, already validated by both the
    /// reader and the validating constructors, so this single binary search is
    /// the aligned-tick rule shared by ranged replay and the rewind player. It
    /// performs no allocation, executes no step and returns `None` when `tick`
    /// is not one of the recorded outcome ticks; the movie start tick is not a
    /// recorded step.
    #[must_use]
    pub(crate) fn recorded_step_index(&self, tick: Tick) -> Option<usize> {
        self.steps
            .binary_search_by_key(&tick.raw(), |step| step.outcome_tick().raw())
            .ok()
    }

    /// Returns the nearest anchor whose recorded step tick is at or before `tick`.
    ///
    /// This is the read-only selection a replay uses to start from the greatest
    /// anchor step index whose recorded outcome tick is `<= tick`; the anchor
    /// table is sorted and its ticks strictly increase, so a binary search is
    /// exact. It returns `None` when the table is empty or when no anchor is at or
    /// before `tick`, which keeps the replay on the embedded checkpoint. It
    /// performs no allocation and executes no step.
    #[must_use]
    pub fn anchor_at_or_before(&self, tick: Tick) -> Option<&MovieAnchor> {
        let index = self.anchors.partition_point(|anchor| anchor.tick <= tick);
        index.checked_sub(1).map(|index| &self.anchors[index])
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
    /// The byte slice, a declared length or a declared count exceeds a format
    /// bound.
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
    /// The container magic is neither `CFMOVIE1` nor `CFMOVIE2`.
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
    /// The declared step count exceeds the format's step limit.
    TooManySteps {
        /// Configured maximum number of steps.
        limit: usize,
        /// Declared or observed count.
        found: u64,
    },
    /// The declared anchor count exceeds [`MAX_ANCHORS`].
    TooManyAnchors {
        /// Configured maximum number of anchors.
        limit: usize,
        /// Declared or observed count.
        found: u64,
    },
    /// The declared embedded checkpoint length exceeds
    /// [`MAX_EMBEDDED_SNAPSHOT_BYTES`].
    EmbeddedSnapshotTooLarge {
        /// Configured maximum in bytes.
        limit: usize,
        /// Declared length.
        found: u64,
    },
    /// One declared anchor snapshot exceeds [`MAX_ANCHOR_SNAPSHOT_BYTES`].
    AnchorSnapshotTooLarge {
        /// Zero-based anchor ordinal in the table.
        anchor_ordinal: usize,
        /// Configured maximum in bytes.
        limit: usize,
        /// Declared length.
        found: u64,
    },
    /// The cumulative anchor snapshot bytes exceed [`MAX_ANCHOR_BYTES`].
    AnchorTableTooLarge {
        /// Configured maximum cumulative bytes.
        limit: usize,
        /// Declared or observed cumulative size.
        found: u64,
    },
    /// An anchor snapshot is malformed, truncated or has a bad checksum.
    InvalidAnchorSnapshot {
        /// Zero-based anchor ordinal in the table, or `0` for a standalone anchor.
        anchor_ordinal: usize,
        /// Preserved session error from [`Snapshot::from_bytes`].
        source: SessionError,
    },
    /// Two anchors are not in strictly increasing step-index order.
    AnchorIndexNotIncreasing {
        /// Zero-based anchor ordinal whose index broke the order.
        anchor_ordinal: usize,
        /// Previous anchor's step index.
        previous: u64,
        /// Step index found in the table.
        found: u64,
    },
    /// An anchor step index is not below the recorded step count.
    AnchorIndexOutOfRange {
        /// Zero-based anchor ordinal.
        anchor_ordinal: usize,
        /// Declared anchor step index.
        index: u64,
        /// Number of recorded steps in the movie.
        step_count: u64,
    },
    /// An anchor snapshot tick disagrees with its recorded step.
    AnchorTickMismatch {
        /// Zero-based anchor ordinal.
        anchor_ordinal: usize,
        /// Recorded step index the anchor claims.
        index: usize,
        /// Tick recorded by the anchor snapshot.
        snapshot_tick: Tick,
        /// Tick recorded by the step expectation.
        recorded_tick: Tick,
    },
    /// An anchor snapshot state digest disagrees with its recorded step.
    AnchorStateHashMismatch {
        /// Zero-based anchor ordinal.
        anchor_ordinal: usize,
        /// Recorded step index the anchor claims.
        index: usize,
        /// State digest recorded by the anchor snapshot.
        snapshot_hash: StateHash,
        /// State digest recorded by the step expectation.
        recorded_hash: StateHash,
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
    /// Importing an anchor checkpoint failed, including backend/configuration
    /// incompatibility decided by the existing compatibility predicate.
    AnchorImport {
        /// Zero-based recorded step the anchor was captured after.
        anchor_step: usize,
        /// Preserved session error.
        source: SessionError,
    },
    /// Capturing, reading or serializing an anchor checkpoint failed while
    /// recording.
    AnchorCaptureFailed {
        /// Zero-based recorded step after which the anchor was captured.
        anchor_step: usize,
        /// Preserved session error.
        source: SessionError,
    },
    /// An anchor interval cannot be applied to the requested input sequence.
    InvalidAnchorInterval {
        /// Requested positive anchor interval.
        interval: usize,
    },
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
            Self::TooManyAnchors { limit, found } => {
                write!(f, "movie declares {found} anchors, limit is {limit}")
            }
            Self::EmbeddedSnapshotTooLarge { limit, found } => {
                write!(f, "embedded checkpoint is {found} bytes, limit is {limit}")
            }
            Self::AnchorSnapshotTooLarge {
                anchor_ordinal,
                limit,
                found,
            } => write!(
                f,
                "anchor {anchor_ordinal} snapshot is {found} bytes, limit is {limit}"
            ),
            Self::AnchorTableTooLarge { limit, found } => {
                write!(f, "anchor snapshots total {found} bytes, limit is {limit}")
            }
            Self::InvalidAnchorSnapshot {
                anchor_ordinal,
                source,
            } => write!(f, "invalid anchor {anchor_ordinal} snapshot: {source}"),
            Self::AnchorIndexNotIncreasing {
                anchor_ordinal,
                previous,
                found,
            } => write!(
                f,
                "anchor {anchor_ordinal} step index {found} does not follow {previous}; \
                 anchor indices must be strictly increasing"
            ),
            Self::AnchorIndexOutOfRange {
                anchor_ordinal,
                index,
                step_count,
            } => write!(
                f,
                "anchor {anchor_ordinal} step index {index} is outside the {step_count} \
                 recorded steps"
            ),
            Self::AnchorTickMismatch {
                anchor_ordinal,
                index,
                snapshot_tick,
                recorded_tick,
            } => write!(
                f,
                "anchor {anchor_ordinal} snapshot tick {snapshot_tick} does not match \
                 recorded step {index} tick {recorded_tick}"
            ),
            Self::AnchorStateHashMismatch {
                anchor_ordinal,
                index,
                snapshot_hash,
                recorded_hash,
            } => write!(
                f,
                "anchor {anchor_ordinal} snapshot state hash {snapshot_hash} does not match \
                 recorded step {index} state hash {recorded_hash}"
            ),
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
            Self::AnchorImport {
                anchor_step,
                source,
            } => write!(
                f,
                "cannot import the anchor after recorded step {anchor_step}: {source}"
            ),
            Self::AnchorCaptureFailed {
                anchor_step,
                source,
            } => write!(
                f,
                "cannot capture the anchor after recorded step {anchor_step}: {source}"
            ),
            Self::InvalidAnchorInterval { interval } => write!(
                f,
                "anchor interval {interval} cannot be used with an empty input sequence"
            ),
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
            | Self::AnchorImport { source: error, .. }
            | Self::InvalidAnchorSnapshot { source: error, .. }
            | Self::AnchorCaptureFailed { source: error, .. }
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

/// Returns whether `input` is exactly a v1 frame, i.e. only its port-0 mask is set.
///
/// The v1 step record is 44 bytes and carries one `u32` button mask; encoding a
/// multi-device frame there would silently drop ports 1–3, the axes and the
/// keyboard. A v1 path therefore rejects such a frame as a schema-2 request.
fn is_v1_input(input: InputFrame) -> bool {
    input == InputFrame::new(input.buttons())
}

/// Rejects multi-device step inputs on the v1 construction and recording paths.
fn reject_v1_device_steps(steps: &[MovieStep]) -> Result<(), MovieError> {
    if steps.iter().any(|step| !is_v1_input(step.input)) {
        return Err(MovieError::UnsupportedInputSchema {
            found: V2_MOVIE_INPUT_SCHEMA_VERSION,
            supported: MOVIE_INPUT_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Rejects multi-device inputs on the v1 recording path before any step executes.
fn reject_v1_device_inputs(inputs: &[InputFrame]) -> Result<(), MovieError> {
    if inputs.iter().any(|input| !is_v1_input(*input)) {
        return Err(MovieError::UnsupportedInputSchema {
            found: V2_MOVIE_INPUT_SCHEMA_VERSION,
            supported: MOVIE_INPUT_SCHEMA_VERSION,
        });
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

mod v2 {
    //! v2 movie branch: anchor table, schema-2 step records and v2 recording.
    //!
    //! This internal submodule is the version branch of the single reader/writer in
    //! [`super`]; it is not a second codec. [`super::Movie::from_bytes`] verifies the
    //! trailing checksum, reads the exact magic and delegates `CFMOVIE2` bodies here;
    //! [`super::Movie::to_bytes`] delegates a v2 value here. The embedded checkpoint
    //! and every anchor snapshot are still parsed only by `Snapshot::from_bytes` and
    //! encoded only by `Snapshot::to_bytes`, and backend/configuration compatibility
    //! is still decided only by the existing snapshot import path.
    //!
    //! The layout, limits and typed rejection categories are those of
    //! [ADR-0006](../../../../docs/adr/0006-movie-format-v2-anchors-and-multi-device-input.md).
    //! Every declared count and length is checked against its v2 limit and against the
    //! bytes that actually remain before a buffer is allocated or a slice is taken,
    //! using checked arithmetic; arbitrary bytes never panic.

    use chronofork_backend_api::StepOutcome;
    use chronofork_core::codec::{Reader, Writer};
    use chronofork_core::{InputFrame, SessionId, StateHash, Tick, hash_bytes};

    use crate::error::SessionError;
    use crate::manager::SessionManager;
    use crate::snapshot::Snapshot;

    use super::{
        ANCHOR_RECORD_OVERHEAD, MAX_ANCHOR_BYTES, MAX_ANCHOR_SNAPSHOT_BYTES, MAX_ANCHORS,
        MAX_EMBEDDED_SNAPSHOT_BYTES, MAX_MOVIE_BYTES, MAX_NAME_BYTES, MAX_V2_STEPS, Movie,
        MovieAnchor, MovieError, MovieMeta, MovieStep, V2_FIXED_OVERHEAD, V2_MAGIC,
        V2_MOVIE_FORMAT_VERSION, V2_MOVIE_INPUT_SCHEMA_VERSION, V2_STEP_RECORD_LEN, codec_error,
        validate_name, validate_steps,
    };

    /// An anchor that has been parsed but not yet range-checked against the steps.
    ///
    /// The declared index stays a `u64` until the step count is known, so no
    /// conversion to `usize` happens before the range check.
    struct PendingAnchor {
        step_index: u64,
        tick: Tick,
        state_hash: StateHash,
        snapshot_bytes: Vec<u8>,
    }

    impl Movie {
        /// Builds a v2 movie from validated metadata, a canonical checkpoint, an
        /// ordered anchor table and schema-2 steps.
        ///
        /// The embedded checkpoint and every anchor snapshot must be exactly
        /// [`Snapshot::to_bytes`] output; each is decoded once by
        /// [`Snapshot::from_bytes`]. This is the single v2 validating construction
        /// path and it applies the same checks as the v2 reader: count and size
        /// limits, strictly increasing anchor indices, index below the step count,
        /// anchor snapshot tick equal to the recorded outcome tick and anchor
        /// snapshot state digest equal to the recorded expected digest. It then hands
        /// the value to [`Movie::to_bytes`], so the writer never emits a movie it
        /// could not read back.
        pub fn from_v2_parts(
            meta: MovieMeta,
            snapshot_bytes: Vec<u8>,
            anchors: Vec<MovieAnchor>,
            steps: Vec<MovieStep>,
        ) -> Result<Self, MovieError> {
            let snapshot = Snapshot::from_bytes(&snapshot_bytes)
                .map_err(MovieError::InvalidEmbeddedSnapshot)?;
            validate_steps(&steps, snapshot.tick())?;
            // Counts and sizes are checked before binding, as the reader does, and
            // again by the encoder that produces the bytes.
            let name_len = meta.name.as_ref().map_or(0, String::len);
            let anchor_lengths: Vec<usize> = anchors
                .iter()
                .map(|anchor| anchor.snapshot_bytes.len())
                .collect();
            checked_len(name_len, snapshot_bytes.len(), &anchor_lengths, steps.len())?;
            validate_anchor_table(&anchors, &steps)?;
            let movie = Self {
                meta,
                snapshot_bytes,
                start_tick: snapshot.tick(),
                start_state_hash: snapshot.state_hash(),
                format_version: V2_MOVIE_FORMAT_VERSION,
                input_schema_version: V2_MOVIE_INPUT_SCHEMA_VERSION,
                anchors,
                steps,
            };
            movie.to_bytes()?;
            Ok(movie)
        }

        /// Records a v2 input sequence from `source` without advancing it.
        ///
        /// The source checkpoint is captured through the existing checkpoint path and
        /// imported into a separate fresh session exactly as [`Movie::record`] does;
        /// the inputs are applied to that imported session and every outcome becomes a
        /// schema-2 expectation record. The source tick, state digest and mutable
        /// state are unchanged (the checkpoint record stored by
        /// [`SessionManager::snapshot`] is the only permitted source-side effect).
        ///
        /// With `anchor_interval = n > 0`, an anchor is captured through
        /// [`SessionManager::snapshot`] and [`SessionManager::snapshot_bytes`] after
        /// recorded steps `n - 1, 2n - 1, ...` while the index is below the step count;
        /// each anchor is the canonical checkpoint for the state after that step and
        /// must match its recorded tick and state digest. `n = 0` records no anchors.
        /// The interval is not stored. A failed step, checkpoint capture, snapshot
        /// read or byte-budget check returns a typed error and no movie.
        pub fn record_v2(
            manager: &mut SessionManager,
            source: SessionId,
            meta: MovieMeta,
            inputs: &[InputFrame],
            anchor_interval: usize,
        ) -> Result<Self, MovieError> {
            if inputs.len() > MAX_V2_STEPS {
                return Err(MovieError::TooManySteps {
                    limit: MAX_V2_STEPS,
                    found: inputs.len() as u64,
                });
            }
            if anchor_interval > 0 {
                if inputs.is_empty() {
                    return Err(MovieError::InvalidAnchorInterval {
                        interval: anchor_interval,
                    });
                }
                let predicted = inputs.len() / anchor_interval;
                if predicted > MAX_ANCHORS {
                    return Err(MovieError::TooManyAnchors {
                        limit: MAX_ANCHORS,
                        found: predicted as u64,
                    });
                }
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
            let mut anchors = Vec::new();
            let mut anchor_bytes_total = 0_usize;
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
                if anchor_interval > 0 && (input_index + 1) % anchor_interval == 0 {
                    capture_anchor(
                        manager,
                        session,
                        input_index,
                        outcome,
                        &mut anchors,
                        &mut anchor_bytes_total,
                    )?;
                }
            }
            Self::from_v2_parts(meta, snapshot_bytes, anchors, steps)
        }
    }

    /// Captures one deterministic anchor in the separate recording session.
    fn capture_anchor(
        manager: &mut SessionManager,
        session: SessionId,
        anchor_step: usize,
        outcome: StepOutcome,
        anchors: &mut Vec<MovieAnchor>,
        anchor_bytes_total: &mut usize,
    ) -> Result<(), MovieError> {
        let anchor_ordinal = anchors.len();
        let snapshot_id =
            manager
                .snapshot(session)
                .map_err(|source| MovieError::AnchorCaptureFailed {
                    anchor_step,
                    source,
                })?;
        let (snapshot_tick, snapshot_hash) = {
            let record = manager
                .snapshot_record(session, snapshot_id)
                .map_err(|source| MovieError::AnchorCaptureFailed {
                    anchor_step,
                    source,
                })?;
            (record.tick(), record.state_hash())
        };
        let snapshot_bytes = manager
            .snapshot_bytes(session, snapshot_id)
            .map_err(|source| MovieError::AnchorCaptureFailed {
                anchor_step,
                source,
            })?;
        if snapshot_tick != outcome.tick {
            return Err(MovieError::AnchorTickMismatch {
                anchor_ordinal,
                index: anchor_step,
                snapshot_tick,
                recorded_tick: outcome.tick,
            });
        }
        if snapshot_hash != outcome.state_hash {
            return Err(MovieError::AnchorStateHashMismatch {
                anchor_ordinal,
                index: anchor_step,
                snapshot_hash,
                recorded_hash: outcome.state_hash,
            });
        }
        let length = snapshot_bytes.len();
        if length > MAX_ANCHOR_SNAPSHOT_BYTES {
            return Err(MovieError::AnchorSnapshotTooLarge {
                anchor_ordinal,
                limit: MAX_ANCHOR_SNAPSHOT_BYTES,
                found: length as u64,
            });
        }
        let total =
            anchor_bytes_total
                .checked_add(length)
                .ok_or(MovieError::AnchorTableTooLarge {
                    limit: MAX_ANCHOR_BYTES,
                    found: u64::MAX,
                })?;
        if total > MAX_ANCHOR_BYTES {
            return Err(MovieError::AnchorTableTooLarge {
                limit: MAX_ANCHOR_BYTES,
                found: total as u64,
            });
        }
        anchors.push(MovieAnchor::with_ordinal(
            anchor_ordinal,
            anchor_step,
            snapshot_bytes,
        )?);
        *anchor_bytes_total = total;
        Ok(())
    }

    /// Encodes a v2 movie into its canonical, checksummed bytes.
    pub(super) fn encode(movie: &Movie) -> Result<Vec<u8>, MovieError> {
        let name_len = movie.meta.name.as_ref().map_or(0, String::len);
        let anchor_lengths: Vec<usize> = movie
            .anchors
            .iter()
            .map(|anchor| anchor.snapshot_bytes.len())
            .collect();
        let total = checked_len(
            name_len,
            movie.snapshot_bytes.len(),
            &anchor_lengths,
            movie.steps.len(),
        )?;
        let mut output = Writer::with_capacity(total);
        output.write_bytes(&V2_MAGIC);
        output.write_u16_le(V2_MOVIE_FORMAT_VERSION);
        output.write_u16_le(V2_MOVIE_INPUT_SCHEMA_VERSION);
        output.write_u64_le(movie.meta.movie_id);
        // `checked_len` rejected more than `MAX_NAME_BYTES` (256), so the conversion
        // to `u16` is exact.
        output.write_u16_le(name_len as u16);
        if let Some(name) = &movie.meta.name {
            output.write_bytes(name.as_bytes());
        }
        output.write_u64_le(movie.meta.parent_movie_id.unwrap_or(0));
        output.write_u64_le(movie.start_tick.raw());
        output.write_u64_le(movie.snapshot_bytes.len() as u64);
        output.write_bytes(&movie.snapshot_bytes);
        output.write_u64_le(movie.anchors.len() as u64);
        for anchor in &movie.anchors {
            output.write_u64_le(anchor.step_index as u64);
            output.write_u64_le(anchor.snapshot_bytes.len() as u64);
            output.write_bytes(&anchor.snapshot_bytes);
        }
        output.write_u64_le(movie.steps.len() as u64);
        for step in &movie.steps {
            output.write_u64_le(step.outcome_tick.raw());
            for port in step.input.ports() {
                output.write_u32_le(port);
            }
            for axis in step.input.axes() {
                output.write_bytes(&axis.to_le_bytes());
            }
            output.write_u64_le(step.input.keys());
            output.write_bytes(step.state_hash.as_bytes());
        }
        let checksum = hash_bytes(output.as_slice());
        output.write_bytes(checksum.as_bytes());
        Ok(output.into_bytes())
    }

    /// Decodes and validates a v2 body positioned just after the magic.
    pub(super) fn decode(reader: &mut Reader<'_>, total_len: usize) -> Result<Movie, MovieError> {
        if total_len < V2_FIXED_OVERHEAD {
            return Err(MovieError::TooShort {
                minimum: V2_FIXED_OVERHEAD,
                found: total_len,
            });
        }
        let format_version = reader.read_u16_le().map_err(codec_error)?;
        if format_version != V2_MOVIE_FORMAT_VERSION {
            return Err(MovieError::UnsupportedFormatVersion {
                found: format_version,
                supported: V2_MOVIE_FORMAT_VERSION,
            });
        }
        let input_schema = reader.read_u16_le().map_err(codec_error)?;
        if input_schema != V2_MOVIE_INPUT_SCHEMA_VERSION {
            return Err(MovieError::UnsupportedInputSchema {
                found: input_schema,
                supported: V2_MOVIE_INPUT_SCHEMA_VERSION,
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
        let snapshot_len = match usize::try_from(snapshot_len_u64) {
            Ok(length) if length <= MAX_EMBEDDED_SNAPSHOT_BYTES => length,
            _ => {
                return Err(MovieError::EmbeddedSnapshotTooLarge {
                    limit: MAX_EMBEDDED_SNAPSHOT_BYTES,
                    found: snapshot_len_u64,
                });
            }
        };
        // The anchor count and step count fields must still fit before the checkpoint
        // is sliced or copied.
        let required = snapshot_len
            .checked_add(8 + 8)
            .ok_or(MovieError::Truncated)?;
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
        let anchor_count_u64 = reader.read_u64_le().map_err(codec_error)?;
        let anchor_count = match usize::try_from(anchor_count_u64) {
            Ok(count) if count <= MAX_ANCHORS => count,
            _ => {
                return Err(MovieError::TooManyAnchors {
                    limit: MAX_ANCHORS,
                    found: anchor_count_u64,
                });
            }
        };
        // The whole anchor framing plus the step-count field must still fit before the
        // anchor vector is reserved.
        let framing = anchor_count
            .checked_mul(ANCHOR_RECORD_OVERHEAD)
            .ok_or(MovieError::Truncated)?;
        let required = framing.checked_add(8).ok_or(MovieError::Truncated)?;
        if reader.remaining() < required {
            return Err(MovieError::Truncated);
        }
        let mut pending: Vec<PendingAnchor> = Vec::with_capacity(anchor_count);
        let mut previous_index: Option<u64> = None;
        let mut anchor_bytes_total = 0_usize;
        for anchor_ordinal in 0..anchor_count {
            let step_index = reader.read_u64_le().map_err(codec_error)?;
            check_anchor_order(anchor_ordinal, previous_index, step_index)?;
            previous_index = Some(step_index);
            let length_u64 = reader.read_u64_le().map_err(codec_error)?;
            let length = match usize::try_from(length_u64) {
                Ok(length) if length <= MAX_ANCHOR_SNAPSHOT_BYTES => length,
                _ => {
                    return Err(MovieError::AnchorSnapshotTooLarge {
                        anchor_ordinal,
                        limit: MAX_ANCHOR_SNAPSHOT_BYTES,
                        found: length_u64,
                    });
                }
            };
            anchor_bytes_total =
                anchor_bytes_total
                    .checked_add(length)
                    .ok_or(MovieError::AnchorTableTooLarge {
                        limit: MAX_ANCHOR_BYTES,
                        found: u64::MAX,
                    })?;
            if anchor_bytes_total > MAX_ANCHOR_BYTES {
                return Err(MovieError::AnchorTableTooLarge {
                    limit: MAX_ANCHOR_BYTES,
                    found: anchor_bytes_total as u64,
                });
            }
            // At least the step-count field must still follow the anchor snapshot.
            let required = length.checked_add(8).ok_or(MovieError::Truncated)?;
            if reader.remaining() < required {
                return Err(MovieError::Truncated);
            }
            let bytes = reader.read_exact(length).map_err(codec_error)?.to_vec();
            let anchor_snapshot = Snapshot::from_bytes(&bytes).map_err(|source| {
                MovieError::InvalidAnchorSnapshot {
                    anchor_ordinal,
                    source,
                }
            })?;
            pending.push(PendingAnchor {
                step_index,
                tick: anchor_snapshot.tick(),
                state_hash: anchor_snapshot.state_hash(),
                snapshot_bytes: bytes,
            });
        }
        let step_count_u64 = reader.read_u64_le().map_err(codec_error)?;
        let step_count = match usize::try_from(step_count_u64) {
            Ok(count) if count <= MAX_V2_STEPS => count,
            _ => {
                return Err(MovieError::TooManySteps {
                    limit: MAX_V2_STEPS,
                    found: step_count_u64,
                });
            }
        };
        // Bounded by `MAX_V2_STEPS × 72`; the remaining bytes are checked before the
        // step vector is reserved.
        let record_bytes = step_count
            .checked_mul(V2_STEP_RECORD_LEN)
            .ok_or(MovieError::Truncated)?;
        if reader.remaining() < record_bytes {
            return Err(MovieError::Truncated);
        }
        let mut steps = Vec::with_capacity(step_count);
        for _ in 0..step_count {
            let outcome_tick = Tick::from_raw(reader.read_u64_le().map_err(codec_error)?);
            let ports = [
                reader.read_u32_le().map_err(codec_error)?,
                reader.read_u32_le().map_err(codec_error)?,
                reader.read_u32_le().map_err(codec_error)?,
                reader.read_u32_le().map_err(codec_error)?,
            ];
            let axes = [
                i16::from_le_bytes(reader.read_array::<2>().map_err(codec_error)?),
                i16::from_le_bytes(reader.read_array::<2>().map_err(codec_error)?),
                i16::from_le_bytes(reader.read_array::<2>().map_err(codec_error)?),
                i16::from_le_bytes(reader.read_array::<2>().map_err(codec_error)?),
            ];
            let keys = reader.read_u64_le().map_err(codec_error)?;
            let state_hash = StateHash::from_bytes(reader.read_array::<32>().map_err(codec_error)?);
            steps.push(MovieStep::new(
                outcome_tick,
                InputFrame::with_devices(ports, axes, keys),
                state_hash,
            ));
        }
        validate_steps(&steps, start_tick)?;
        let mut anchors = Vec::with_capacity(pending.len());
        for (anchor_ordinal, anchor) in pending.into_iter().enumerate() {
            check_anchor_range(anchor_ordinal, anchor.step_index, step_count_u64)?;
            let step_index = usize::try_from(anchor.step_index).map_err(|_| {
                MovieError::AnchorIndexOutOfRange {
                    anchor_ordinal,
                    index: anchor.step_index,
                    step_count: step_count_u64,
                }
            })?;
            anchors.push(MovieAnchor {
                step_index,
                tick: anchor.tick,
                state_hash: anchor.state_hash,
                snapshot_bytes: anchor.snapshot_bytes,
            });
        }
        validate_anchor_table(&anchors, &steps)?;
        reader.finish().map_err(codec_error)?;
        Ok(Movie {
            meta: MovieMeta {
                movie_id,
                name,
                parent_movie_id,
            },
            snapshot_bytes: snapshot_bytes.to_vec(),
            start_tick,
            start_state_hash: snapshot.state_hash(),
            format_version: V2_MOVIE_FORMAT_VERSION,
            input_schema_version: V2_MOVIE_INPUT_SCHEMA_VERSION,
            anchors,
            steps,
        })
    }

    /// Validates the encoded v2 length against every count and size limit.
    ///
    /// This is the single v2 size validator used by the writer and by the tests; it
    /// allocates nothing and takes the anchor snapshot lengths rather than the
    /// anchors, so a limit violation is proven before any buffer is sized from a
    /// declared value.
    pub(super) fn checked_len(
        name_len: usize,
        snapshot_len: usize,
        anchor_lengths: &[usize],
        step_count: usize,
    ) -> Result<usize, MovieError> {
        if name_len > MAX_NAME_BYTES {
            return Err(MovieError::InvalidName(format!(
                "movie name is {name_len} bytes, limit is {MAX_NAME_BYTES}"
            )));
        }
        if step_count > MAX_V2_STEPS {
            return Err(MovieError::TooManySteps {
                limit: MAX_V2_STEPS,
                found: step_count as u64,
            });
        }
        if snapshot_len > MAX_EMBEDDED_SNAPSHOT_BYTES {
            return Err(MovieError::EmbeddedSnapshotTooLarge {
                limit: MAX_EMBEDDED_SNAPSHOT_BYTES,
                found: snapshot_len as u64,
            });
        }
        if anchor_lengths.len() > MAX_ANCHORS {
            return Err(MovieError::TooManyAnchors {
                limit: MAX_ANCHORS,
                found: anchor_lengths.len() as u64,
            });
        }
        let mut anchor_bytes_total = 0_usize;
        for (anchor_ordinal, length) in anchor_lengths.iter().copied().enumerate() {
            if length > MAX_ANCHOR_SNAPSHOT_BYTES {
                return Err(MovieError::AnchorSnapshotTooLarge {
                    anchor_ordinal,
                    limit: MAX_ANCHOR_SNAPSHOT_BYTES,
                    found: length as u64,
                });
            }
            anchor_bytes_total =
                anchor_bytes_total
                    .checked_add(length)
                    .ok_or(MovieError::AnchorTableTooLarge {
                        limit: MAX_ANCHOR_BYTES,
                        found: u64::MAX,
                    })?;
            if anchor_bytes_total > MAX_ANCHOR_BYTES {
                return Err(MovieError::AnchorTableTooLarge {
                    limit: MAX_ANCHOR_BYTES,
                    found: anchor_bytes_total as u64,
                });
            }
        }
        let overflow = MovieError::FileTooLarge {
            limit: MAX_MOVIE_BYTES,
            found: u64::MAX,
        };
        let anchor_framing = anchor_lengths
            .len()
            .checked_mul(ANCHOR_RECORD_OVERHEAD)
            .ok_or(overflow.clone())?;
        let step_bytes = step_count
            .checked_mul(V2_STEP_RECORD_LEN)
            .ok_or(overflow.clone())?;
        let total = V2_FIXED_OVERHEAD
            .checked_add(name_len)
            .and_then(|value| value.checked_add(snapshot_len))
            .and_then(|value| value.checked_add(anchor_framing))
            .and_then(|value| value.checked_add(anchor_bytes_total))
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

    /// Validates that anchor indices are strictly increasing as they are read.
    fn check_anchor_order(
        anchor_ordinal: usize,
        previous: Option<u64>,
        found: u64,
    ) -> Result<(), MovieError> {
        if let Some(previous) = previous {
            if found <= previous {
                return Err(MovieError::AnchorIndexNotIncreasing {
                    anchor_ordinal,
                    previous,
                    found,
                });
            }
        }
        Ok(())
    }

    /// Validates that an anchor step index is below the recorded step count.
    fn check_anchor_range(
        anchor_ordinal: usize,
        index: u64,
        step_count: u64,
    ) -> Result<(), MovieError> {
        if index >= step_count {
            return Err(MovieError::AnchorIndexOutOfRange {
                anchor_ordinal,
                index,
                step_count,
            });
        }
        Ok(())
    }

    /// The single anchor validator shared by the v2 reader and the v2 constructor.
    ///
    /// It checks ordering, range and step binding in anchor order; tick and state
    /// digest are read from the already parsed anchor snapshot and compared with the
    /// recorded expectation of its step.
    fn validate_anchor_table(
        anchors: &[MovieAnchor],
        steps: &[MovieStep],
    ) -> Result<(), MovieError> {
        let mut previous: Option<u64> = None;
        for (anchor_ordinal, anchor) in anchors.iter().enumerate() {
            let index = anchor.step_index as u64;
            check_anchor_order(anchor_ordinal, previous, index)?;
            check_anchor_range(anchor_ordinal, index, steps.len() as u64)?;
            let step = &steps[anchor.step_index];
            if anchor.tick != step.outcome_tick() {
                return Err(MovieError::AnchorTickMismatch {
                    anchor_ordinal,
                    index: anchor.step_index,
                    snapshot_tick: anchor.tick,
                    recorded_tick: step.outcome_tick(),
                });
            }
            if anchor.state_hash != step.state_hash() {
                return Err(MovieError::AnchorStateHashMismatch {
                    anchor_ordinal,
                    index: anchor.step_index,
                    snapshot_hash: anchor.state_hash,
                    recorded_hash: step.state_hash(),
                });
            }
            previous = Some(index);
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;

        use chronofork_backend_api::{BackendId, BackendSnapshot};
        use chronofork_core::{InputFrame, SessionId, SnapshotId, StateHash, Tick, hash_bytes};
        use chronofork_test_backend::{SyntheticBackendFactory, SyntheticProgram};

        use super::{
            MAX_ANCHOR_BYTES, MAX_ANCHOR_SNAPSHOT_BYTES, MAX_ANCHORS, MAX_EMBEDDED_SNAPSHOT_BYTES,
            MAX_MOVIE_BYTES, MAX_NAME_BYTES, MAX_V2_STEPS, checked_len,
        };
        use crate::SessionError;
        use crate::manager::SessionManager;
        use crate::movie::{
            ANCHOR_RECORD_OVERHEAD, CHECKSUM_LEN, Movie, MovieAnchor, MovieError, MovieMeta,
            MovieStep, V2_FIXED_OVERHEAD, V2_MOVIE_FORMAT_VERSION, V2_MOVIE_INPUT_SCHEMA_VERSION,
            V2_STEP_RECORD_LEN,
        };
        use crate::snapshot::Snapshot;

        const BACKEND_ID: &str = "chronofork.test.synthetic";
        const BACKEND_VERSION: &str = "0.1.0";

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

        fn snapshot_bytes(tick: u64, state: u8, snapshot_id: u64) -> Vec<u8> {
            fabricated_snapshot(tick, state, snapshot_id)
                .to_bytes()
                .expect("fabricated snapshot encodes")
        }

        fn meta() -> MovieMeta {
            MovieMeta::new(7, Some("v2".to_owned()), Some(3)).expect("movie metadata")
        }

        fn step(tick: u64, input: InputFrame, state: u8) -> MovieStep {
            MovieStep::new(
                Tick::from_raw(tick),
                input,
                StateHash::from_bytes([state; 32]),
            )
        }

        fn anchor(index: usize, tick: u64, state: u8, snapshot_id: u64) -> MovieAnchor {
            MovieAnchor::new(index, snapshot_bytes(tick, state, snapshot_id)).expect("valid anchor")
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

        fn sample_movie() -> Movie {
            Movie::from_v2_parts(
                meta(),
                snapshot_bytes(0, 0x00, 1),
                vec![anchor(0, 1, 0x21, 2), anchor(2, 3, 0x23, 3)],
                sample_steps(),
            )
            .expect("sample v2 movie")
        }

        fn single_anchor_movie() -> Movie {
            Movie::from_v2_parts(
                meta(),
                snapshot_bytes(0, 0x00, 1),
                vec![anchor(0, 1, 0x21, 2)],
                vec![step(1, InputFrame::new(1), 0x21)],
            )
            .expect("single-anchor v2 movie")
        }

        fn empty_movie() -> Movie {
            Movie::from_v2_parts(meta(), snapshot_bytes(0, 0x00, 1), Vec::new(), Vec::new())
                .expect("zero-step v2 movie")
        }

        fn resign(bytes: &mut [u8]) {
            let body_len = bytes.len() - CHECKSUM_LEN;
            let checksum = hash_bytes(&bytes[..body_len]);
            bytes[body_len..].copy_from_slice(checksum.as_bytes());
        }

        fn mutate(bytes: &[u8], offset: usize, value: &[u8]) -> Vec<u8> {
            let mut bytes = bytes.to_vec();
            bytes[offset..offset + value.len()].copy_from_slice(value);
            resign(&mut bytes);
            bytes
        }

        fn name_len(movie: &Movie) -> usize {
            movie.meta().name().map_or(0, str::len)
        }

        fn anchor_count_offset(movie: &Movie) -> usize {
            22 + name_len(movie) + 8 + 8 + 8 + movie.snapshot_bytes().len()
        }

        fn anchor_offset(movie: &Movie, ordinal: usize) -> usize {
            let mut offset = anchor_count_offset(movie) + 8;
            for anchor in &movie.anchors()[..ordinal] {
                offset += ANCHOR_RECORD_OVERHEAD + anchor.snapshot_bytes().len();
            }
            offset
        }

        fn step_count_offset(movie: &Movie) -> usize {
            anchor_offset(movie, movie.anchors().len())
        }

        fn record_offset(movie: &Movie) -> usize {
            step_count_offset(movie) + 8
        }

        fn encoded(movie: &Movie) -> Vec<u8> {
            movie.to_bytes().expect("encode v2 movie")
        }

        #[test]
        fn round_trip_preserves_metadata_anchors_and_device_steps() {
            let movie = sample_movie();
            assert_eq!(movie.format_version(), V2_MOVIE_FORMAT_VERSION);
            assert_eq!(movie.input_schema_version(), V2_MOVIE_INPUT_SCHEMA_VERSION);
            assert_eq!(movie.anchors().len(), 2);
            assert_eq!(movie.anchors()[0].step_index(), 0);
            assert_eq!(movie.anchors()[1].step_index(), 2);
            assert_eq!(
                movie.anchors()[1].snapshot_bytes(),
                snapshot_bytes(3, 0x23, 3).as_slice()
            );
            assert_eq!(movie.meta().movie_id(), 7);
            assert_eq!(movie.meta().name(), Some("v2"));
            assert_eq!(movie.meta().parent_movie_id(), Some(3));
            assert_eq!(movie.steps(), sample_steps());

            let bytes = encoded(&movie);
            let expected = V2_FIXED_OVERHEAD
                + name_len(&movie)
                + movie.snapshot_bytes().len()
                + movie
                    .anchors()
                    .iter()
                    .map(|anchor| ANCHOR_RECORD_OVERHEAD + anchor.snapshot_bytes().len())
                    .sum::<usize>()
                + movie.steps().len() * V2_STEP_RECORD_LEN;
            assert_eq!(bytes.len(), expected, "exact canonical v2 size");

            let decoded = Movie::from_bytes(&bytes).expect("decode v2 movie");
            assert_eq!(decoded, movie);
            assert_eq!(decoded.steps(), sample_steps());
            assert_eq!(decoded.to_bytes().expect("re-encode"), bytes);
        }

        #[test]
        fn v2_encoding_is_deterministic() {
            let first = encoded(&sample_movie());
            std::thread::yield_now();
            let second = encoded(&sample_movie());
            assert_eq!(first, second);
            assert_eq!(hash_bytes(&first), hash_bytes(&second));
            assert_eq!(&first[..8], b"CFMOVIE2");
        }

        #[test]
        fn zero_step_zero_anchor_v2_movie_round_trips_with_exact_size() {
            let movie = empty_movie();
            assert_eq!(movie.step_count(), 0);
            assert!(movie.anchors().is_empty());
            let bytes = encoded(&movie);
            assert_eq!(
                bytes.len(),
                V2_FIXED_OVERHEAD + name_len(&movie) + movie.snapshot_bytes().len()
            );
            assert_eq!(Movie::from_bytes(&bytes).expect("decode"), movie);

            let unnamed = Movie::from_v2_parts(
                MovieMeta::new(9, None, None).expect("metadata"),
                snapshot_bytes(4, 0x44, 5),
                Vec::new(),
                Vec::new(),
            )
            .expect("zero-step anonymous movie");
            assert_eq!(
                encoded(&unnamed).len(),
                V2_FIXED_OVERHEAD + unnamed.snapshot_bytes().len()
            );
            assert_eq!(
                Movie::from_bytes(&encoded(&unnamed)).expect("decode"),
                unnamed
            );
        }

        #[test]
        fn anchor_selection_returns_the_nearest_anchor_at_or_before() {
            let movie = Movie::from_v2_parts(
                meta(),
                snapshot_bytes(0, 0x00, 1),
                vec![
                    anchor(0, 1, 0x21, 2),
                    anchor(2, 3, 0x23, 3),
                    anchor(4, 5, 0x25, 4),
                ],
                vec![
                    step(1, InputFrame::new(1), 0x21),
                    step(2, InputFrame::new(2), 0x22),
                    step(3, InputFrame::new(3), 0x23),
                    step(4, InputFrame::new(4), 0x24),
                    step(5, InputFrame::new(5), 0x25),
                ],
            )
            .expect("anchored movie");
            assert!(movie.anchor_at_or_before(Tick::ZERO).is_none());
            assert_eq!(
                movie
                    .anchor_at_or_before(Tick::from_raw(1))
                    .map(|anchor| anchor.step_index()),
                Some(0)
            );
            assert_eq!(
                movie
                    .anchor_at_or_before(Tick::from_raw(2))
                    .map(|anchor| anchor.step_index()),
                Some(0)
            );
            assert_eq!(
                movie
                    .anchor_at_or_before(Tick::from_raw(3))
                    .map(|anchor| anchor.step_index()),
                Some(2)
            );
            assert_eq!(
                movie
                    .anchor_at_or_before(Tick::from_raw(4))
                    .map(|anchor| anchor.step_index()),
                Some(2)
            );
            assert_eq!(
                movie
                    .anchor_at_or_before(Tick::from_raw(u64::MAX))
                    .map(|anchor| anchor.step_index()),
                Some(4)
            );
        }

        #[test]
        fn non_uniform_anchor_tables_are_accepted() {
            let movie = Movie::from_v2_parts(
                meta(),
                snapshot_bytes(0, 0x00, 1),
                vec![
                    anchor(0, 1, 0x21, 2),
                    anchor(1, 2, 0x22, 3),
                    anchor(3, 4, 0x24, 4),
                ],
                sample_steps(),
            )
            .expect("non-uniform anchors");
            let decoded = Movie::from_bytes(&encoded(&movie)).expect("decode");
            assert_eq!(decoded.anchors(), movie.anchors());
        }

        #[test]
        fn anchor_binding_rejections_are_typed() {
            let embedded = snapshot_bytes(0, 0x00, 1);
            let steps = sample_steps();

            let not_increasing = vec![anchor(1, 2, 0x22, 2), anchor(1, 2, 0x22, 3)];
            assert!(matches!(
                Movie::from_v2_parts(meta(), embedded.clone(), not_increasing, steps.clone()),
                Err(MovieError::AnchorIndexNotIncreasing {
                    anchor_ordinal: 1,
                    previous: 1,
                    found: 1
                })
            ));

            let out_of_range = vec![anchor(4, 5, 0x25, 2)];
            assert!(matches!(
                Movie::from_v2_parts(meta(), embedded.clone(), out_of_range, steps.clone()),
                Err(MovieError::AnchorIndexOutOfRange {
                    anchor_ordinal: 0,
                    index: 4,
                    step_count: 4
                })
            ));

            let zero_step_with_anchor = vec![anchor(0, 1, 0x21, 2)];
            assert!(matches!(
                Movie::from_v2_parts(meta(), embedded.clone(), zero_step_with_anchor, Vec::new()),
                Err(MovieError::AnchorIndexOutOfRange {
                    anchor_ordinal: 0,
                    index: 0,
                    step_count: 0
                })
            ));

            let tick_mismatch = vec![anchor(0, 2, 0x22, 2)];
            assert!(matches!(
                Movie::from_v2_parts(meta(), embedded.clone(), tick_mismatch, steps.clone()),
                Err(MovieError::AnchorTickMismatch {
                    anchor_ordinal: 0,
                    index: 0,
                    snapshot_tick,
                    recorded_tick
                }) if snapshot_tick == Tick::from_raw(2) && recorded_tick == Tick::from_raw(1)
            ));

            let hash_mismatch = vec![anchor(0, 1, 0xee, 2)];
            assert!(matches!(
                Movie::from_v2_parts(meta(), embedded, hash_mismatch, steps),
                Err(MovieError::AnchorStateHashMismatch {
                    anchor_ordinal: 0,
                    index: 0,
                    ..
                })
            ));
        }

        #[test]
        fn corrupt_anchor_snapshots_are_rejected_by_the_anchor_constructor() {
            let mut corrupt = snapshot_bytes(1, 0x21, 2);
            let last = corrupt.len() - 1;
            corrupt[last] ^= 0xff;
            assert!(matches!(
                MovieAnchor::new(0, corrupt),
                Err(MovieError::InvalidAnchorSnapshot {
                    anchor_ordinal: 0,
                    source: SessionError::InvalidSnapshot(_)
                })
            ));

            let mut truncated = snapshot_bytes(1, 0x21, 2);
            truncated.pop();
            assert!(matches!(
                MovieAnchor::new(0, truncated),
                Err(MovieError::InvalidAnchorSnapshot {
                    source: SessionError::InvalidSnapshot(_),
                    ..
                })
            ));
        }

        #[test]
        fn v2_magic_version_and_schema_matrix_is_reject_only() {
            let bytes = encoded(&sample_movie());
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
                Movie::from_bytes(&mutate(&bytes, 8, &3_u16.to_le_bytes())),
                Err(MovieError::UnsupportedFormatVersion {
                    found: 3,
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
                Movie::from_bytes(&mutate(&bytes, 10, &3_u16.to_le_bytes())),
                Err(MovieError::UnsupportedInputSchema {
                    found: 3,
                    supported: 2
                })
            ));
            // A CFTMOVIE1 magic keeps the retained v1 path and its v1 error.
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
        }

        #[test]
        fn v2_metadata_and_start_tick_rejections_are_typed() {
            let movie = sample_movie();
            let bytes = encoded(&movie);
            let parent = 22 + name_len(&movie);
            let start_tick = parent + 8;
            let name_start = 22;

            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, 12, &0_u64.to_le_bytes())),
                Err(MovieError::InvalidMovieId)
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    parent,
                    &movie.meta().movie_id().to_le_bytes()
                )),
                Err(MovieError::InvalidParentMovieId)
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, 20, &257_u16.to_le_bytes())),
                Err(MovieError::InvalidName(_))
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, name_start, &[0xff])),
                Err(MovieError::InvalidName(_))
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, name_start, &[0x01])),
                Err(MovieError::InvalidName(_))
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, start_tick, &1_u64.to_le_bytes())),
                Err(MovieError::StartTickMismatch { declared, snapshot })
                    if declared == Tick::from_raw(1) && snapshot == Tick::ZERO
            ));
        }

        #[test]
        fn v2_embedded_checkpoint_rejections_are_typed_and_wrapped() {
            let movie = sample_movie();
            let bytes = encoded(&movie);
            let parent = 22 + name_len(&movie);
            let snapshot_len = parent + 16;
            let snapshot_start = snapshot_len + 8;

            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    snapshot_len,
                    &(MAX_EMBEDDED_SNAPSHOT_BYTES as u64 + 1).to_le_bytes()
                )),
                Err(MovieError::EmbeddedSnapshotTooLarge { limit, .. })
                    if limit == MAX_EMBEDDED_SNAPSHOT_BYTES
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, snapshot_len, &u64::MAX.to_le_bytes())),
                Err(MovieError::EmbeddedSnapshotTooLarge { .. })
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, snapshot_len, &1000_u64.to_le_bytes())),
                Err(MovieError::Truncated)
            ));

            // A checksum-bad embedded checkpoint is wrapped, preserving the session
            // error rather than flattening it to a string.
            let mut corrupt = bytes.clone();
            let embedded_last = snapshot_start + movie.snapshot_bytes().len() - 1;
            corrupt[embedded_last] ^= 0xff;
            resign(&mut corrupt);
            assert!(matches!(
                Movie::from_bytes(&corrupt),
                Err(MovieError::InvalidEmbeddedSnapshot(
                    SessionError::InvalidSnapshot(_)
                ))
            ));

            // An unsupported snapshot envelope version is IncompatibleSnapshot.
            let mut incompatible = bytes.clone();
            incompatible[snapshot_start + 8..snapshot_start + 10]
                .copy_from_slice(&2_u16.to_le_bytes());
            let snapshot_body = snapshot_start + movie.snapshot_bytes().len() - CHECKSUM_LEN;
            let snapshot_checksum = hash_bytes(&incompatible[snapshot_start..snapshot_body]);
            incompatible[snapshot_body..snapshot_start + movie.snapshot_bytes().len()]
                .copy_from_slice(snapshot_checksum.as_bytes());
            resign(&mut incompatible);
            assert!(matches!(
                Movie::from_bytes(&incompatible),
                Err(MovieError::InvalidEmbeddedSnapshot(
                    SessionError::IncompatibleSnapshot(_)
                ))
            ));
        }

        #[test]
        fn v2_declared_counts_and_lengths_are_bounded_before_allocation() {
            let movie = sample_movie();
            let bytes = encoded(&movie);
            let anchor_count = anchor_count_offset(&movie);
            let step_count = step_count_offset(&movie);

            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    anchor_count,
                    &(MAX_ANCHORS as u64 + 1).to_le_bytes()
                )),
                Err(MovieError::TooManyAnchors { limit, .. }) if limit == MAX_ANCHORS
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, anchor_count, &u64::MAX.to_le_bytes())),
                Err(MovieError::TooManyAnchors { .. })
            ));
            // A zero-anchor, zero-step movie has only the step-count field left, so a
            // declared anchor cannot even fit its framing.
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &encoded(&empty_movie()),
                    anchor_count_offset(&empty_movie()),
                    &1_u64.to_le_bytes()
                )),
                Err(MovieError::Truncated)
            ));

            let first_anchor = anchor_offset(&movie, 0);
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    first_anchor + 8,
                    &(MAX_ANCHOR_SNAPSHOT_BYTES as u64 + 1).to_le_bytes()
                )),
                Err(MovieError::AnchorSnapshotTooLarge {
                    anchor_ordinal: 0,
                    limit,
                    ..
                }) if limit == MAX_ANCHOR_SNAPSHOT_BYTES
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, first_anchor + 8, &u64::MAX.to_le_bytes())),
                Err(MovieError::AnchorSnapshotTooLarge { .. })
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    first_anchor + 8,
                    &1_000_000_u64.to_le_bytes()
                )),
                Err(MovieError::Truncated)
            ));

            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    step_count,
                    &(MAX_V2_STEPS as u64 + 1).to_le_bytes()
                )),
                Err(MovieError::TooManySteps { limit, .. }) if limit == MAX_V2_STEPS
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, step_count, &u64::MAX.to_le_bytes())),
                Err(MovieError::TooManySteps { .. })
            ));
            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, step_count, &5_u64.to_le_bytes())),
                Err(MovieError::Truncated)
            ));
        }

        #[test]
        fn v2_anchor_binding_rejections_at_decode_are_typed() {
            let movie = sample_movie();
            let bytes = encoded(&movie);
            let second_anchor = anchor_offset(&movie, 1);

            assert!(matches!(
                Movie::from_bytes(&mutate(&bytes, second_anchor, &0_u64.to_le_bytes())),
                Err(MovieError::AnchorIndexNotIncreasing {
                    anchor_ordinal: 1,
                    previous: 0,
                    found: 0
                })
            ));

            let single = single_anchor_movie();
            let single_bytes = encoded(&single);
            let single_anchor_offset = anchor_offset(&single, 0);
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &single_bytes,
                    single_anchor_offset,
                    &1_u64.to_le_bytes()
                )),
                Err(MovieError::AnchorIndexOutOfRange {
                    anchor_ordinal: 0,
                    index: 1,
                    step_count: 1
                })
            ));

            // An anchor on a zero-step movie is out of range; binding is validated
            // before trailing bytes are inspected.
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &bytes,
                    step_count_offset(&movie),
                    &0_u64.to_le_bytes()
                )),
                Err(MovieError::AnchorIndexOutOfRange {
                    anchor_ordinal: 0,
                    index: 0,
                    step_count: 0
                })
            ));

            // A valid tick sequence whose step tick no longer matches the anchor.
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &single_bytes,
                    record_offset(&single),
                    &2_u64.to_le_bytes()
                )),
                Err(MovieError::AnchorTickMismatch {
                    anchor_ordinal: 0,
                    index: 0,
                    snapshot_tick,
                    recorded_tick
                }) if snapshot_tick == Tick::from_raw(1) && recorded_tick == Tick::from_raw(2)
            ));

            // The recorded state digest no longer matches the anchor snapshot.
            assert!(matches!(
                Movie::from_bytes(&mutate(
                    &single_bytes,
                    record_offset(&single) + 40,
                    &[0xee; 32]
                )),
                Err(MovieError::AnchorStateHashMismatch {
                    anchor_ordinal: 0,
                    index: 0,
                    ..
                })
            ));

            // A corrupt anchor snapshot names its table ordinal.
            let mut corrupt = bytes.clone();
            let anchor_snapshot_len = movie.anchors()[1].snapshot_bytes().len();
            let corrupt_last = second_anchor + ANCHOR_RECORD_OVERHEAD + anchor_snapshot_len - 1;
            corrupt[corrupt_last] ^= 0xff;
            resign(&mut corrupt);
            assert!(matches!(
                Movie::from_bytes(&corrupt),
                Err(MovieError::InvalidAnchorSnapshot {
                    anchor_ordinal: 1,
                    source: SessionError::InvalidSnapshot(_)
                })
            ));
        }

        #[test]
        fn v2_trailing_bytes_truncation_and_arbitrary_bytes_are_rejected() {
            let bytes = encoded(&sample_movie());
            let mut appended = bytes.clone();
            appended.push(0);
            assert!(matches!(
                Movie::from_bytes(&appended),
                Err(MovieError::ChecksumMismatch)
            ));

            let mut body = bytes[..bytes.len() - CHECKSUM_LEN].to_vec();
            body.push(0);
            body.extend_from_slice(&[0; CHECKSUM_LEN]);
            resign(&mut body);
            assert!(matches!(
                Movie::from_bytes(&body),
                Err(MovieError::TrailingBytes { count: 1 })
            ));

            for cut in 0..bytes.len() {
                assert!(
                    Movie::from_bytes(&bytes[..cut]).is_err(),
                    "truncated at {cut} bytes must be rejected"
                );
            }

            let mut state = 0x2545_f491_4f6c_dd1d_u64;
            for length in 0..=2048 {
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
        }

        #[test]
        fn v2_checked_len_enforces_every_documented_limit() {
            assert_eq!(V2_FIXED_OVERHEAD, 94);
            assert_eq!(V2_STEP_RECORD_LEN, 72);
            assert_eq!(ANCHOR_RECORD_OVERHEAD, 16);
            assert!(matches!(
                checked_len(MAX_NAME_BYTES + 1, 0, &[], 0),
                Err(MovieError::InvalidName(_))
            ));
            assert!(matches!(
                checked_len(0, 0, &[], MAX_V2_STEPS + 1),
                Err(MovieError::TooManySteps { limit, .. }) if limit == MAX_V2_STEPS
            ));
            assert!(matches!(
                checked_len(0, MAX_EMBEDDED_SNAPSHOT_BYTES + 1, &[], 0),
                Err(MovieError::EmbeddedSnapshotTooLarge { limit, .. })
                    if limit == MAX_EMBEDDED_SNAPSHOT_BYTES
            ));
            let too_many = vec![0_usize; MAX_ANCHORS + 1];
            assert!(matches!(
                checked_len(0, 0, &too_many, 0),
                Err(MovieError::TooManyAnchors { limit, .. }) if limit == MAX_ANCHORS
            ));
            assert!(matches!(
                checked_len(0, 0, &[MAX_ANCHOR_SNAPSHOT_BYTES + 1], 0),
                Err(MovieError::AnchorSnapshotTooLarge {
                    anchor_ordinal: 0,
                    limit,
                    ..
                }) if limit == MAX_ANCHOR_SNAPSHOT_BYTES
            ));
            let half = MAX_ANCHOR_BYTES / 2 + 1;
            assert!(matches!(
                checked_len(0, 0, &[half, half], 0),
                Err(MovieError::AnchorTableTooLarge { limit, .. }) if limit == MAX_ANCHOR_BYTES
            ));
            assert_eq!(checked_len(0, 0, &[], 0), Ok(V2_FIXED_OVERHEAD));
        }

        #[test]
        fn the_maximal_documented_v2_combination_fits_the_file_bound() {
            let anchor_length = MAX_ANCHOR_BYTES / MAX_ANCHORS;
            let anchor_lengths = vec![anchor_length; MAX_ANCHORS];
            let total = checked_len(
                MAX_NAME_BYTES,
                MAX_EMBEDDED_SNAPSHOT_BYTES,
                &anchor_lengths,
                MAX_V2_STEPS,
            )
            .expect("the maximal documented v2 combination must fit");
            assert_eq!(total, 262_489_438);
            assert!(total < MAX_MOVIE_BYTES);
        }

        fn new_manager() -> SessionManager {
            SessionManager::new(Arc::new(SyntheticBackendFactory::default()))
        }

        fn recorded_movie(inputs: &[InputFrame], anchor_interval: usize) -> Movie {
            let mut manager = new_manager();
            let source = manager.create_session().expect("source session");
            for _ in 0..3 {
                manager
                    .step(source, InputFrame::new(3))
                    .expect("advance source");
            }
            Movie::record_v2(&mut manager, source, meta(), inputs, anchor_interval)
                .expect("record v2 movie")
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

        #[test]
        fn v2_recording_captures_canonical_anchors_and_leaves_the_source_unchanged() {
            let mut manager = new_manager();
            let source = manager.create_session().expect("source session");
            for _ in 0..3 {
                manager
                    .step(source, InputFrame::new(3))
                    .expect("advance source");
            }
            let before = manager.inspect(source).expect("inspect source").machine;
            let inputs = device_inputs();
            let movie = Movie::record_v2(&mut manager, source, meta(), &inputs, 2)
                .expect("record anchored movie");
            let after = manager.inspect(source).expect("inspect source").machine;
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
                let snapshot = Snapshot::from_bytes(anchor.snapshot_bytes())
                    .expect("canonical anchor snapshot");
                assert_eq!(
                    snapshot.to_bytes().expect("re-encode anchor").as_slice(),
                    anchor.snapshot_bytes(),
                    "an anchor stores the exact canonical Snapshot::to_bytes output"
                );
            }

            let mut verifying = new_manager();
            let report = movie
                .verify(&mut verifying)
                .expect("recorded movie verifies");
            assert_eq!(report.steps, inputs.len());
            assert_eq!(report.final_state_hash, movie.final_state_hash());
            let decoded = Movie::from_bytes(&movie.to_bytes().expect("encode")).expect("decode");
            assert_eq!(decoded, movie);
        }

        #[test]
        fn v2_recording_is_byte_deterministic_across_fresh_managers() {
            let inputs = device_inputs();
            let first = recorded_movie(&inputs, 2);
            std::thread::yield_now();
            let second = recorded_movie(&inputs, 2);
            assert_eq!(
                first.to_bytes().expect("encode first"),
                second.to_bytes().expect("encode second")
            );
            assert_eq!(
                first
                    .anchors()
                    .iter()
                    .map(MovieAnchor::step_index)
                    .collect::<Vec<_>>(),
                vec![1, 3]
            );
        }

        #[test]
        fn v2_anchor_interval_policies_are_deterministic_and_typed() {
            let inputs = device_inputs();
            let unanchored = recorded_movie(&inputs, 0);
            assert!(unanchored.anchors().is_empty());
            assert_eq!(unanchored.format_version(), V2_MOVIE_FORMAT_VERSION);

            let too_wide = recorded_movie(&inputs, inputs.len() + 1);
            assert!(
                too_wide.anchors().is_empty(),
                "interval above the step count places no anchor"
            );

            let every_step = recorded_movie(&inputs, 1);
            assert_eq!(
                every_step
                    .anchors()
                    .iter()
                    .map(MovieAnchor::step_index)
                    .collect::<Vec<_>>(),
                vec![0, 1, 2, 3, 4]
            );

            let mut manager = new_manager();
            let source = manager.create_session().expect("source session");
            assert!(matches!(
                Movie::record_v2(&mut manager, source, meta(), &[], 2),
                Err(MovieError::InvalidAnchorInterval { interval: 2 })
            ));
            let zero_step =
                Movie::record_v2(&mut manager, source, meta(), &[], 0).expect("zero-step v2 movie");
            assert_eq!(zero_step.step_count(), 0);
            assert!(zero_step.anchors().is_empty());

            let too_many_inputs = vec![InputFrame::NEUTRAL; MAX_ANCHORS + 1];
            assert!(matches!(
                Movie::record_v2(&mut manager, source, meta(), &too_many_inputs, 1),
                Err(MovieError::TooManyAnchors { limit, .. }) if limit == MAX_ANCHORS
            ));
        }

        #[test]
        fn a_failed_v2_recording_is_typed_and_leaves_the_source_unchanged() {
            let halt_factory = SyntheticBackendFactory::new(
                SyntheticProgram::new(vec![0xff]).expect("halting program"),
            );
            let mut manager = SessionManager::new(Arc::new(halt_factory));
            let source = manager.create_session().expect("source session");
            let before = manager.inspect(source).expect("inspect").machine;
            let result = Movie::record_v2(
                &mut manager,
                source,
                meta(),
                &[InputFrame::NEUTRAL, InputFrame::NEUTRAL],
                1,
            );
            assert!(
                matches!(
                    result,
                    Err(MovieError::RecordFailed {
                        input_index: 0,
                        source: SessionError::Halted(_)
                    })
                ),
                "expected a halted recording"
            );
            let after = manager.inspect(source).expect("inspect").machine;
            assert_eq!(after.tick, before.tick);
            assert_eq!(after.state_hash, before.state_hash);
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

    #[test]
    fn v1_values_stay_format_one_with_no_anchors_and_the_v1_encoded_size() {
        let movie = movie();
        assert_eq!(movie.format_version(), MOVIE_FORMAT_VERSION);
        assert_eq!(movie.input_schema_version(), MOVIE_INPUT_SCHEMA_VERSION);
        assert!(movie.anchors().is_empty());
        assert!(
            movie
                .anchor_at_or_before(Tick::from_raw(u64::MAX))
                .is_none()
        );

        let bytes = movie.to_bytes().expect("encode v1 movie");
        let expected = FIXED_OVERHEAD
            + name_len(&movie)
            + movie.snapshot_bytes().len()
            + movie.step_count() * STEP_RECORD_LEN;
        assert_eq!(bytes.len(), expected);
        assert_eq!(&bytes[..8], b"CFMOVIE1");
        assert_eq!(
            &bytes[8..10],
            &MOVIE_FORMAT_VERSION.to_le_bytes(),
            "v1 keeps format version 1"
        );
        assert_eq!(
            &bytes[10..12],
            &MOVIE_INPUT_SCHEMA_VERSION.to_le_bytes(),
            "v1 keeps schema version 1"
        );
        let decoded = Movie::from_bytes(&bytes).expect("decode v1 movie");
        assert_eq!(decoded, movie);
        assert_eq!(decoded.to_bytes().expect("re-encode"), bytes);
    }

    #[test]
    fn v1_construction_and_recording_reject_multi_device_frames() {
        let device_input = InputFrame::with_devices([0, 1, 0, 0], [0; 4], 0);
        let device_steps = vec![MovieStep::new(
            Tick::from_raw(1),
            device_input,
            StateHash::from_bytes([1; 32]),
        )];
        assert!(matches!(
            Movie::from_parts(meta(), checkpoint_bytes(), device_steps),
            Err(MovieError::UnsupportedInputSchema { found, supported })
                if found == crate::movie::V2_MOVIE_INPUT_SCHEMA_VERSION
                    && supported == MOVIE_INPUT_SCHEMA_VERSION
        ));

        let mut manager =
            crate::SessionManager::new(std::sync::Arc::new(SyntheticBackendFactory::default()));
        let source = manager.create_session().expect("source session");
        assert!(matches!(
            Movie::record(&mut manager, source, meta(), &[device_input]),
            Err(MovieError::UnsupportedInputSchema { found, supported })
                if found == crate::movie::V2_MOVIE_INPUT_SCHEMA_VERSION
                    && supported == MOVIE_INPUT_SCHEMA_VERSION
        ));
    }

    #[test]
    fn recorded_step_index_is_the_single_alignment_lookup() {
        let movie = movie();
        assert_eq!(movie.recorded_step_index(Tick::from_raw(1)), Some(0));
        assert_eq!(movie.recorded_step_index(Tick::from_raw(3)), Some(2));
        assert_eq!(movie.recorded_step_index(Tick::from_raw(4)), Some(3));
        // The movie start tick is not a recorded step.
        assert_eq!(movie.recorded_step_index(Tick::from_raw(0)), None);
        assert_eq!(movie.recorded_step_index(Tick::from_raw(5)), None);

        let gapped = Movie::from_parts(
            meta(),
            checkpoint_bytes(),
            vec![
                MovieStep::new(
                    Tick::from_raw(10),
                    InputFrame::new(0),
                    StateHash::from_bytes([1; 32]),
                ),
                MovieStep::new(
                    Tick::from_raw(20),
                    InputFrame::new(1),
                    StateHash::from_bytes([2; 32]),
                ),
            ],
        )
        .expect("gapped movie");
        assert_eq!(gapped.recorded_step_index(Tick::from_raw(10)), Some(0));
        assert_eq!(gapped.recorded_step_index(Tick::from_raw(20)), Some(1));
        assert_eq!(gapped.recorded_step_index(Tick::from_raw(15)), None);
    }
}
