//! Deterministic input editing, truncation and branch creation.
//!
//! This module implements the library half of
//! [ADR-0008](../../../docs/adr/0008-rerecord-and-branch-editing.md): given a
//! decoded v1 or v2 [`Movie`], an optional set of replacement [`InputFrame`]s for
//! zero-based recorded step indices, an optional truncation index and branch
//! metadata, [`edit_movie`] produces a **new** canonical movie of the source's own
//! format. The source movie value and its bytes are never mutated: the function
//! borrows `&Movie` and returns a value.
//!
//! # Editing model
//!
//! Let the source movie have `n = source.step_count()` steps, edit indices `E` and
//! an optional last retained step `T` (`truncate_after`). The output retains
//! `m = T + 1` steps when truncation is requested and `m = n` otherwise. The
//! earliest affected step is `e = min(E ∪ {T + 1})`; steps `0..e` are the
//! **prefix** and steps `e..m` the **suffix**. The same single loop then:
//!
//! 1. imports the source's embedded checkpoint through the existing
//!    [`SessionManager::create_session_from_snapshot_bytes`] path, so backend,
//!    payload and configuration compatibility stay decided by the existing
//!    compatibility predicate;
//! 2. executes every output step `0..m` once in ascending order in that fresh
//!    session, using the replacement frame at an edited index and the recorded
//!    frame otherwise;
//! 3. verifies each prefix step against its source record and, on success, copies
//!    the source record verbatim, so the prefix bytes are exactly the source
//!    bytes; the first mismatch is the existing typed [`MovieError::TickMismatch`]
//!    or [`MovieError::Divergence`] at its exact index and no movie is produced;
//! 4. records every suffix step from the observed outcome, so every suffix
//!    expectation is recomputed and none is copied from the source;
//! 5. validates and constructs the output with the source's own validating
//!    constructor ([`Movie::from_parts`] for a v1 source,
//!    [`Movie::from_v2_parts`] for a v2 source), which also enforces the inherited
//!    step, anchor and size limits through the single writer.
//!
//! A pure truncation executes only the retained prefix and never executes a
//! dropped step. "Rerecord" here means exactly this deterministic suffix
//! re-simulation: no new emulation mode, no in-place mutation, no persisted
//! counter and no new movie format. Repeated edits form a chain of new artifacts
//! whose only provenance is the immediate parent edge.
//!
//! # Branch metadata
//!
//! The output always carries a non-zero `movie_id`, `parent_movie_id` equal to the
//! source movie id (the passive, never-resolved dangling-parent rule), and the
//! optional name from [`EditMeta`]. The default movie id is the source id checked
//! `+ 1`; an explicit id must be non-zero and differ from the source id. The name
//! is never inherited from the source. Ids are logical labels, not a registry.
//!
//! # Anchors
//!
//! The embedded initial checkpoint is always copied verbatim. [`AnchorPolicy`]
//! decides the explicit anchor table of a v2 output:
//!
//! - [`AnchorPolicy::KeepPrefix`] (default) retains, verbatim and in order, only
//!   the source anchors whose step index is before `e`; anchors at or after `e`
//!   are dropped, because the state they describe no longer exists.
//! - [`AnchorPolicy::Drop`] produces an empty table.
//! - [`AnchorPolicy::Rebuild { interval }`] drops every source anchor and captures
//!   a new canonical anchor after output steps `interval - 1, 2*interval - 1, ...`
//!   through the existing capture path
//!   ([`SessionManager::snapshot`]/[`SessionManager::snapshot_record`]/
//!   [`SessionManager::snapshot_bytes`] and [`crate::MovieAnchor::new`]). Each
//!   rebuilt envelope is re-encoded with a fixed canonical snapshot/origin-session
//!   identity, still through the one snapshot parser and serializer, so its bytes
//!   do not depend on how many sessions the caller's manager created first; the
//!   embedded checkpoint is never re-encoded.
//!
//! Retained and rebuilt anchors are never mixed, no anchor is used as an edit
//! start state, and a v1 source rejects an explicit anchor policy because v1 has
//! no anchor table.
//!
//! # Typed bounds and errors
//!
//! Request validation runs before any checkpoint import, backend step, anchor
//! capture or step-vector allocation: the request rejects an empty set or a
//! duplicate index, and [`edit_movie`] rejects out-of-range or truncated-away
//! indices, a no-op truncation, an unsupported v1 multi-device replacement,
//! branch-id conflicts and overflows, invalid anchor policies and intervals, and a
//! predicted anchor count above [`MAX_ANCHORS`].
//! Device, name, id, anchor-binding and size limits are reported through the
//! existing [`MovieError`] categories. Every error is typed in [`EditError`] and
//! preserves its [`MovieError`]/[`SessionError`] source; a failed edit returns no
//! partial movie.
//!
//! # Determinism, isolation and ownership
//!
//! For identical source bytes, request and metadata, the output bytes are
//! deterministic: nothing from the host clock, paths, process identity, manager
//! session history or entropy enters canonical bytes. Rebuilt anchor envelopes
//! carry a canonical fixed identity, so a prior edit in the same manager cannot
//! change a later edit's bytes. The function registers exactly one imported
//! session in the caller's manager; a rejected request registers none and
//! executes no step.
//! There is one movie codec ([`Movie::from_bytes`]/[`Movie::to_bytes`]),
//! one embedded-checkpoint parser ([`crate::snapshot::Snapshot::from_bytes`]
//! reached through the two existing construction/import paths), one compatibility
//! predicate, one checkpoint import path and one anchor capture path; this module
//! adds no second codec, parser, predicate, replay loop, dependency, `unsafe`,
//! allowance or ignored test.
//!
//! Deferred and not implemented here: in-place editing, scripted input files,
//! timeline insert/delete/reorder, a persisted rerecord counter, mixing retained
//! and rebuilt anchors, CLI device-field editing of ports 1–3/axes/keys (full-frame
//! replacement is library-only), real-hardware input TAS and any emulation,
//! state-hash or payload change. The `chronofork tas edit` process surface is a
//! separate layer; M3 rerecord acceptance rows are executed and verified by their
//! own stage.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use chronofork_core::InputFrame;

use crate::error::SessionError;
use crate::manager::SessionManager;
use crate::movie::{
    AnchorIdentity, MAX_ANCHORS, MOVIE_FORMAT_VERSION, MOVIE_INPUT_SCHEMA_VERSION, Movie,
    MovieError, MovieMeta, MovieStep, V2_MOVIE_FORMAT_VERSION, V2_MOVIE_INPUT_SCHEMA_VERSION,
    capture_anchor,
};

/// How an edit handles the source movie's explicit v2 anchor table.
///
/// The embedded initial checkpoint is never an anchor and is always copied
/// verbatim. A v1 source has no anchor table, so any explicit policy on a v1
/// source is a typed [`EditError::AnchorsUnsupportedForV1`]; the default
/// [`AnchorPolicy::KeepPrefix`] on v1 is simply the empty implicit rule.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnchorPolicy {
    /// Retain each source anchor whose step index is before the earliest affected
    /// step, verbatim and in order; drop every anchor at or after it.
    #[default]
    KeepPrefix,
    /// Drop every source anchor; the output v2 movie has an empty anchor table.
    Drop,
    /// Drop every source anchor and rebuild a table at `interval` spacing.
    ///
    /// A new canonical anchor is captured after output steps `interval - 1,
    /// 2*interval - 1, ...` while the index is below the output step count, so the
    /// output has `output_steps / interval` anchors. The interval is not stored in
    /// the movie. `interval` must satisfy `1 <= interval <= output_steps` and the
    /// predicted count must not exceed [`MAX_ANCHORS`];
    /// [`EditError::InvalidAnchorInterval`] and
    /// [`EditError::TooManyRebuiltAnchors`] report violations before execution.
    Rebuild {
        /// Positive anchor interval in output steps.
        interval: usize,
    },
}

/// Branch metadata for an edit.
///
/// The parent movie id is not part of this value: an edit always sets
/// `parent_movie_id` to the source movie id and never lets a caller misrepresent
/// provenance. The name is never inherited from the source; without
/// [`EditMeta::with_name`] the output has no name.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EditMeta {
    movie_id: Option<u64>,
    name: Option<String>,
    anchor_policy: Option<AnchorPolicy>,
}

impl EditMeta {
    /// Creates branch metadata with the default movie id (source id `+ 1`), no
    /// name and the default [`AnchorPolicy::KeepPrefix`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an explicit output movie id.
    ///
    /// The id must be non-zero and differ from the source movie id; violations are
    /// typed [`EditError`] failures reported before any checkpoint import.
    #[must_use]
    pub fn with_movie_id(mut self, movie_id: u64) -> Self {
        self.movie_id = Some(movie_id);
        self
    }

    /// Sets the optional non-inherited output name.
    ///
    /// The name is validated by [`MovieMeta::new`] through the existing
    /// [`MAX_NAME_BYTES`](crate::MAX_NAME_BYTES) rule: non-empty, at most 256 UTF-8
    /// bytes and free of control characters.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets an explicit anchor policy, overriding the default keep-prefix rule.
    ///
    /// An explicit policy on a v1 source is a typed
    /// [`EditError::AnchorsUnsupportedForV1`].
    #[must_use]
    pub fn with_anchor_policy(mut self, anchor_policy: AnchorPolicy) -> Self {
        self.anchor_policy = Some(anchor_policy);
        self
    }
}

/// A validated edit request: replacement input frames and an optional truncation.
///
/// Indices are zero-based recorded step indices stored as `u64` until
/// [`edit_movie`] range-checks them against the source movie, so no `usize`
/// conversion happens before validation. The request stores at most one frame per
/// distinct index; a duplicate is rejected immediately by
/// [`MovieEdit::with_input`], and out-of-range or truncated-away indices are
/// rejected by [`edit_movie`] before any session is imported or step executed. The
/// library replacement is a full [`InputFrame`]; the CLI's port-0-only editing is a
/// separate layer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MovieEdit {
    edits: BTreeMap<u64, InputFrame>,
    truncate_after: Option<u64>,
}

impl MovieEdit {
    /// Creates an empty edit request.
    ///
    /// An empty request is a typed [`EditError::NoEditsRequested`] rather than a
    /// metadata-only copy: this increment has no branch-copy operation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a full-frame replacement for the recorded step at `index`.
    ///
    /// A duplicate index is a typed [`EditError::DuplicateEditIndex`] instead of a
    /// silent last-wins overwrite.
    pub fn with_input(mut self, index: u64, input: InputFrame) -> Result<Self, EditError> {
        if self.edits.insert(index, input).is_some() {
            return Err(EditError::DuplicateEditIndex { index });
        }
        Ok(self)
    }

    /// Requests truncation after the zero-based step `index`.
    ///
    /// The output retains steps `0..=index` and drops every later step.
    /// [`edit_movie`] requires at least one dropped step, so a no-op truncation is
    /// a typed [`EditError::TruncationNoOp`].
    #[must_use]
    pub fn with_truncate_after(mut self, index: u64) -> Self {
        self.truncate_after = Some(index);
        self
    }
}

/// Per-invocation diagnostics of one successful edit.
///
/// These values describe this operation only. They are not a cumulative rerecord
/// counter, a history record or a movie field: the canonical bytes carry no
/// counter, edit log or format version for editing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditReport {
    /// Movie id of the edited source.
    pub source_movie_id: u64,
    /// Movie id of the new branch artifact.
    pub movie_id: u64,
    /// Parent movie id, always the source movie id.
    pub parent_movie_id: u64,
    /// Output movie format version: `1` or `2`, equal to the source's.
    pub format_version: u16,
    /// Output recorded input schema version: `1` or `2`, equal to the source's.
    pub input_schema_version: u16,
    /// Number of distinct edited steps.
    pub edits: usize,
    /// Last retained step index when truncation was requested.
    pub truncate_after: Option<usize>,
    /// Number of copied and verified prefix steps (`e`).
    pub prefix_steps: usize,
    /// Number of re-simulated suffix steps (`m - e`).
    pub resimulated_steps: usize,
    /// Anchor policy that produced the output table.
    pub anchor_policy: AnchorPolicy,
    /// Number of source anchors retained verbatim.
    pub anchors_retained: usize,
    /// Number of source anchors dropped.
    pub anchors_dropped: usize,
    /// Number of anchors rebuilt during the edit.
    pub anchors_rebuilt: usize,
}

/// A successful edit: the new movie plus its diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditOutcome {
    movie: Movie,
    report: EditReport,
}

impl EditOutcome {
    /// Returns the new, independently validated branch movie.
    #[must_use]
    pub const fn movie(&self) -> &Movie {
        &self.movie
    }

    /// Returns the per-invocation diagnostics.
    #[must_use]
    pub const fn report(&self) -> &EditReport {
        &self.report
    }
}

/// A typed edit rejection or failure.
///
/// Request, identity and anchor-policy variants are produced by validation before
/// any checkpoint import, backend step, anchor capture or step-vector allocation.
/// [`EditError::Movie`] preserves the existing [`MovieError`] categories (device
/// schema, name, id, format, anchor binding and size limits, and the first
/// `TickMismatch`/`Divergence` of a non-reproducing source prefix).
/// [`EditError::Import`] and [`EditError::Step`] preserve the existing
/// [`SessionError`], with the zero-based output step index where applicable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditError {
    /// The request set neither an input replacement nor a truncation.
    NoEditsRequested,
    /// The same zero-based step index was edited more than once.
    DuplicateEditIndex {
        /// Repeated edit index.
        index: u64,
    },
    /// An edit index is not below the source step count.
    EditIndexOutOfRange {
        /// Requested edit index.
        index: u64,
        /// Source recorded step count.
        step_count: u64,
    },
    /// An edit index is retained by nothing because it is beyond the truncation.
    EditBeyondTruncation {
        /// Requested edit index.
        index: u64,
        /// Requested last retained step.
        truncate_after: u64,
    },
    /// A truncation index is not below the source step count.
    TruncateIndexOutOfRange {
        /// Requested truncation index.
        index: u64,
        /// Source recorded step count.
        step_count: u64,
    },
    /// A truncation request would drop no step.
    TruncationNoOp {
        /// Requested truncation index.
        index: u64,
        /// Source recorded step count.
        step_count: u64,
    },
    /// An explicit output movie id equals the source movie id.
    BranchIdEqualsSource {
        /// Conflicting movie id.
        movie_id: u64,
    },
    /// The default output movie id (source id `+ 1`) overflows.
    BranchIdOverflow {
        /// Source movie id.
        source_movie_id: u64,
    },
    /// An explicit anchor policy or interval was requested for a v1 source.
    AnchorsUnsupportedForV1,
    /// An anchor rebuild interval is zero or exceeds the output step count.
    InvalidAnchorInterval {
        /// Requested positive interval.
        interval: usize,
        /// Output step count.
        steps: usize,
    },
    /// An anchor rebuild would produce more anchors than the format allows.
    TooManyRebuiltAnchors {
        /// Configured maximum anchor count.
        limit: usize,
        /// Predicted anchor count.
        found: u64,
    },
    /// An existing movie or anchor rule rejected the request or the output.
    Movie(MovieError),
    /// Importing the source's embedded checkpoint failed.
    Import(SessionError),
    /// A backend or session step failed at a zero-based output step index.
    Step {
        /// Zero-based output step index.
        step_index: usize,
        /// Preserved session error.
        source: SessionError,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEditsRequested => {
                f.write_str("edit request must replace at least one input or truncate after a step")
            }
            Self::DuplicateEditIndex { index } => {
                write!(f, "edit index {index} is repeated")
            }
            Self::EditIndexOutOfRange { index, step_count } => write!(
                f,
                "edit index {index} is out of range for a movie with {step_count} step(s)"
            ),
            Self::EditBeyondTruncation {
                index,
                truncate_after,
            } => write!(
                f,
                "edit index {index} is beyond truncate-after {truncate_after}"
            ),
            Self::TruncateIndexOutOfRange { index, step_count } => write!(
                f,
                "truncate-after {index} is out of range for a movie with {step_count} step(s)"
            ),
            Self::TruncationNoOp { index, step_count } => write!(
                f,
                "truncate-after {index} drops no step from a movie with {step_count} step(s)"
            ),
            Self::BranchIdEqualsSource { movie_id } => {
                write!(f, "branch movie id {movie_id} equals the source movie id")
            }
            Self::BranchIdOverflow { source_movie_id } => write!(
                f,
                "default branch movie id overflows for source movie id {source_movie_id}"
            ),
            Self::AnchorsUnsupportedForV1 => f.write_str(
                "a v1 movie has no anchor table; an explicit anchor policy is unsupported",
            ),
            Self::InvalidAnchorInterval { interval, steps } => write!(
                f,
                "anchor rebuild interval {interval} is invalid for {steps} output step(s)"
            ),
            Self::TooManyRebuiltAnchors { limit, found } => {
                write!(
                    f,
                    "anchor rebuild would produce {found} anchors, limit is {limit}"
                )
            }
            Self::Movie(source) => write!(f, "{source}"),
            Self::Import(source) => write!(f, "importing the embedded checkpoint failed: {source}"),
            Self::Step { step_index, source } => {
                write!(f, "output step {step_index} failed: {source}")
            }
        }
    }
}

impl Error for EditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Movie(source) => Some(source),
            Self::Import(source) | Self::Step { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A validated edit plan: everything [`edit_movie`] needs after request
/// validation.
struct EditPlan {
    meta: MovieMeta,
    output_steps: usize,
    earliest_affected: usize,
    truncate_after: Option<usize>,
    edits: BTreeMap<usize, InputFrame>,
    anchor_policy: AnchorPolicy,
}

/// Edits or truncates a decoded movie and returns a new canonical branch movie.
///
/// The source `movie` is borrowed immutably and is never mutated or re-encoded;
/// the output is a value validated by the source's own constructor and encoded by
/// the single writer. The request is validated in full before the embedded
/// checkpoint is imported, so a rejected request registers no session in `manager`
/// and executes no step. On success exactly one fresh imported session is
/// registered in `manager`; pre-existing sessions are never touched.
///
/// The unchanged prefix is executed and compared against the source records and
/// copied verbatim, the suffix is re-simulated deterministically from the
/// replacement frames, and every suffix expectation is recomputed. [`Movie::verify`]
/// remains the separate complete verification path for the returned movie; the edit
/// does not run a second full verification pass. The cost is one backend step per
/// output step from the embedded checkpoint, so a late edit approaches a full
/// replay.
pub fn edit_movie(
    movie: &Movie,
    manager: &mut SessionManager,
    edit: &MovieEdit,
    meta: &EditMeta,
) -> Result<EditOutcome, EditError> {
    let plan = validate_edit(movie, edit, meta)?;
    let source_movie_id = movie.meta().movie_id();
    let output_steps = plan.output_steps;
    let earliest_affected = plan.earliest_affected;
    let truncate_after = plan.truncate_after;
    let anchor_policy = plan.anchor_policy;
    let edit_count = plan.edits.len();

    let mut anchors = match anchor_policy {
        AnchorPolicy::KeepPrefix => movie
            .anchors()
            .iter()
            .filter(|anchor| anchor.step_index() < earliest_affected)
            .cloned()
            .collect::<Vec<_>>(),
        AnchorPolicy::Drop | AnchorPolicy::Rebuild { .. } => Vec::new(),
    };
    let anchors_retained = anchors.len();
    let rebuild_interval = match anchor_policy {
        AnchorPolicy::Rebuild { interval } => Some(interval),
        AnchorPolicy::KeepPrefix | AnchorPolicy::Drop => None,
    };

    let session = manager
        .create_session_from_snapshot_bytes(movie.snapshot_bytes())
        .map_err(EditError::Import)?;
    let mut steps = Vec::with_capacity(output_steps);
    let mut anchor_bytes_total = 0_usize;
    for (index, source_step) in movie.steps().iter().take(output_steps).enumerate() {
        let input = plan
            .edits
            .get(&index)
            .copied()
            .unwrap_or_else(|| source_step.input());
        let outcome = manager
            .step_verified(session, input)
            .map_err(|source| EditError::Step {
                step_index: index,
                source,
            })?;
        if outcome.halted && index + 1 < output_steps {
            return Err(EditError::Step {
                step_index: index,
                source: SessionError::Halted(session),
            });
        }
        if index < earliest_affected {
            if outcome.tick != source_step.outcome_tick() {
                return Err(EditError::Movie(MovieError::TickMismatch {
                    input_index: index,
                    expected: source_step.outcome_tick(),
                    observed: outcome.tick,
                }));
            }
            if outcome.state_hash != source_step.state_hash() {
                return Err(EditError::Movie(MovieError::Divergence {
                    tick: source_step.outcome_tick(),
                    input_index: index,
                    expected: source_step.state_hash(),
                    observed: outcome.state_hash,
                }));
            }
            steps.push(*source_step);
        } else {
            steps.push(MovieStep::new(outcome.tick, input, outcome.state_hash));
        }
        if let Some(interval) = rebuild_interval {
            if (index + 1) % interval == 0 {
                capture_anchor(
                    manager,
                    session,
                    index,
                    outcome,
                    AnchorIdentity::Canonical,
                    &mut anchors,
                    &mut anchor_bytes_total,
                )
                .map_err(EditError::Movie)?;
            }
        }
    }

    let anchors_dropped = movie.anchors().len() - anchors_retained;
    let anchors_rebuilt = if rebuild_interval.is_some() {
        anchors.len()
    } else {
        0
    };
    let movie = match movie.format_version() {
        V2_MOVIE_FORMAT_VERSION => {
            Movie::from_v2_parts(plan.meta, movie.snapshot_bytes().to_vec(), anchors, steps)
                .map_err(EditError::Movie)?
        }
        MOVIE_FORMAT_VERSION => {
            Movie::from_parts(plan.meta, movie.snapshot_bytes().to_vec(), steps)
                .map_err(EditError::Movie)?
        }
        found => {
            return Err(EditError::Movie(MovieError::UnsupportedFormatVersion {
                found,
                supported: MOVIE_FORMAT_VERSION,
            }));
        }
    };
    let report = EditReport {
        source_movie_id,
        movie_id: movie.meta().movie_id(),
        // The edit always sets the parent edge to the source movie id.
        parent_movie_id: source_movie_id,
        format_version: movie.format_version(),
        input_schema_version: movie.input_schema_version(),
        edits: edit_count,
        truncate_after,
        prefix_steps: earliest_affected,
        resimulated_steps: output_steps - earliest_affected,
        anchor_policy,
        anchors_retained,
        anchors_dropped,
        anchors_rebuilt,
    };
    Ok(EditOutcome { movie, report })
}

/// Validates an edit request against a decoded source and resolves its metadata.
///
/// The check order is request presence, truncation bounds, edit indices
/// (duplicates are already rejected by the request builder), v1 device
/// encodability, anchor policy and interval budgets, and finally branch identity
/// and name. Every check is pure arithmetic or bounded
/// metadata construction: no checkpoint is decoded, no session is registered, no
/// step executes and no output-sized vector is allocated.
fn validate_edit(movie: &Movie, edit: &MovieEdit, meta: &EditMeta) -> Result<EditPlan, EditError> {
    if edit.edits.is_empty() && edit.truncate_after.is_none() {
        return Err(EditError::NoEditsRequested);
    }
    let step_count = movie.step_count();
    // A validated movie's step count fits the format's `usize` limit.
    let step_count_u64 = step_count as u64;
    let truncate_after = match edit.truncate_after {
        Some(index) => {
            if index >= step_count_u64 {
                return Err(EditError::TruncateIndexOutOfRange {
                    index,
                    step_count: step_count_u64,
                });
            }
            // `index < step_count` and `step_count` fits `usize`, so both the
            // conversion and `+ 1` are exact.
            let index = index as usize;
            if index + 1 == step_count {
                return Err(EditError::TruncationNoOp {
                    index: index as u64,
                    step_count: step_count_u64,
                });
            }
            Some(index)
        }
        None => None,
    };
    let output_steps = truncate_after.map_or(step_count, |index| index + 1);
    let mut edits = BTreeMap::new();
    for (&index, &input) in &edit.edits {
        if index >= step_count_u64 {
            return Err(EditError::EditIndexOutOfRange {
                index,
                step_count: step_count_u64,
            });
        }
        if let Some(truncate) = truncate_after {
            if index > truncate as u64 {
                return Err(EditError::EditBeyondTruncation {
                    index,
                    truncate_after: truncate as u64,
                });
            }
        }
        edits.insert(index as usize, input);
    }
    if movie.format_version() == MOVIE_FORMAT_VERSION {
        for input in edits.values() {
            if *input != InputFrame::new(input.buttons()) {
                return Err(EditError::Movie(MovieError::UnsupportedInputSchema {
                    found: V2_MOVIE_INPUT_SCHEMA_VERSION,
                    supported: MOVIE_INPUT_SCHEMA_VERSION,
                }));
            }
        }
    }
    let anchor_policy = match meta.anchor_policy {
        None => AnchorPolicy::KeepPrefix,
        Some(policy) => {
            if movie.format_version() != V2_MOVIE_FORMAT_VERSION {
                return Err(EditError::AnchorsUnsupportedForV1);
            }
            if let AnchorPolicy::Rebuild { interval } = policy {
                if interval == 0 || interval > output_steps {
                    return Err(EditError::InvalidAnchorInterval {
                        interval,
                        steps: output_steps,
                    });
                }
                let predicted = output_steps / interval;
                if predicted > MAX_ANCHORS {
                    return Err(EditError::TooManyRebuiltAnchors {
                        limit: MAX_ANCHORS,
                        found: predicted as u64,
                    });
                }
            }
            policy
        }
    };
    let source_movie_id = movie.meta().movie_id();
    let movie_id = match meta.movie_id {
        Some(movie_id) => movie_id,
        None => source_movie_id
            .checked_add(1)
            .ok_or(EditError::BranchIdOverflow { source_movie_id })?,
    };
    if movie_id == source_movie_id {
        return Err(EditError::BranchIdEqualsSource { movie_id });
    }
    let movie_meta = MovieMeta::new(movie_id, meta.name.clone(), Some(source_movie_id))
        .map_err(EditError::Movie)?;
    // The request is non-empty, so at least one of its two parts contributes the
    // earliest affected step.
    let earliest_affected = edits
        .keys()
        .copied()
        .chain(truncate_after.map(|index| index + 1))
        .min()
        .ok_or(EditError::NoEditsRequested)?;
    Ok(EditPlan {
        meta: movie_meta,
        output_steps,
        earliest_affected,
        truncate_after,
        edits,
        anchor_policy,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_core::InputFrame;
    use chronofork_test_backend::SyntheticBackendFactory;

    use super::{EditError, EditMeta, MovieEdit, edit_movie};
    use crate::manager::SessionManager;
    use crate::movie::{Movie, MovieError, MovieMeta};

    fn recorded_movie() -> Movie {
        let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
        let source = manager.create_session().expect("source session");
        Movie::record(
            &mut manager,
            source,
            MovieMeta::new(1, None, None).expect("movie metadata"),
            &[InputFrame::new(1), InputFrame::new(2)],
        )
        .expect("record movie")
    }

    #[test]
    fn duplicate_edit_index_is_rejected_by_the_request() {
        let request = MovieEdit::new().with_input(1, InputFrame::new(1));
        let request = request.expect("first edit index is distinct");
        let error = request
            .with_input(1, InputFrame::new(2))
            .expect_err("a duplicate edit index must be rejected");
        assert!(matches!(error, EditError::DuplicateEditIndex { index: 1 }));
    }

    #[test]
    fn empty_request_is_a_typed_rejection_without_a_session() {
        let movie = recorded_movie();
        let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
        let error = edit_movie(&movie, &mut manager, &MovieEdit::new(), &EditMeta::new())
            .expect_err("an empty edit request must be rejected");
        assert!(matches!(error, EditError::NoEditsRequested));
        assert!(manager.is_empty());
    }

    #[test]
    fn v1_source_rejects_a_multi_device_replacement_before_import() {
        let movie = recorded_movie();
        let mut manager = SessionManager::new(Arc::new(SyntheticBackendFactory::default()));
        let request = MovieEdit::new()
            .with_input(0, InputFrame::with_devices([1, 2, 0, 0], [0; 4], 0))
            .expect("distinct edit index");
        let error = edit_movie(&movie, &mut manager, &request, &EditMeta::new())
            .expect_err("a v1 movie cannot encode a multi-device frame");
        assert!(matches!(
            error,
            EditError::Movie(MovieError::UnsupportedInputSchema {
                found: 2,
                supported: 1
            })
        ));
        assert!(manager.is_empty());
    }
}
