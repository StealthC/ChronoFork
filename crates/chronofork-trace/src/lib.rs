//! Versioned structured execution traces for ChronoFork backends.
//!
//! A [`Trace`] is a host observation artifact: a validated [`TraceMeta`] plus a
//! bounded, contiguous window of [`TraceRecord`] values. It is never canonical
//! machine state, so tracing on must not change a snapshot payload, state hash,
//! configuration hash, payload version or movie byte. See
//! [ADR-0011](../../../docs/adr/0011-tracing-and-scripting.md).
//!
//! The crate owns exactly one implementation of each rule:
//!
//! - [`capture()`] drives any [`chronofork_backend_api::Backend`] through
//!   inspection, memory reads and optional debug observation.
//! - [`Trace::to_bytes`] / [`Trace::from_bytes`] are the canonical `CFTRACE1`
//!   v1 codec. Import is reject-only; there is no migration or reinterpretation.
//! - [`Trace::query`], [`Trace::compare`] and [`Trace::to_tsv`] are the only
//!   query, first-divergence and one-way text export paths.
//!
//! # Format
//!
//! The canonical stream is big-endian with no padding and no trailing bytes:
//! the eight magic bytes [`TRACE_MAGIC`], the versioned header defined by
//! ADR-0011 §3.2.1 and one fixed-layout record per retained entry. Every limit
//! below is documented, checked before allocation and reported as a typed
//! [`TraceError`] with a stable [`TraceError::code`].

#![cfg_attr(test, allow(clippy::expect_used))]

mod codec;
mod trace;
mod tsv;

pub mod capture;
pub mod error;
pub mod meta;
pub mod query;
pub mod record;

pub use capture::{TraceConfig, TraceStart, TraceStop, capture};
pub use error::TraceError;
pub use meta::{TraceMeta, TraceStopReason};
pub use query::{
    TraceComparison, TraceDivergence, TraceDivergenceReason, TraceQuery, TraceQueryResult,
};
pub use record::{InterruptPhase, TraceKind, TraceKindMask, TraceRecord};
pub use trace::Trace;

/// Version of the canonical trace format accepted and produced by this crate.
pub const TRACE_FORMAT_VERSION: u16 = 1;

/// Magic prefix of a canonical trace file.
pub const TRACE_MAGIC: &[u8] = b"CFTRACE1";

/// Largest accepted encoded trace, in bytes.
pub const MAX_TRACE_BYTES: usize = 64 * 1024 * 1024;

/// Largest accepted record count in one trace.
pub const MAX_TRACE_RECORDS: usize = 1_000_000;

/// Largest accepted ring capacity in one trace or capture configuration.
pub const MAX_TRACE_CAPACITY: usize = 1_000_000;

/// Ring capacity used by a capture that does not choose one.
pub const DEFAULT_TRACE_CAPACITY: usize = 4_096;

/// Largest accepted backend identifier, in UTF-8 bytes.
pub const MAX_BACKEND_ID_BYTES: usize = 128;

/// Largest accepted backend version string, in UTF-8 bytes.
pub const MAX_BACKEND_VERSION_BYTES: usize = 64;

/// Largest accepted step count for one capture.
pub const MAX_CAPTURE_STEPS: u64 = 1_000_000;

/// Backend event-ring capacity used by a capture that does not choose one.
pub const DEFAULT_EVENT_CAPACITY: usize = 1_024;

/// Largest accepted backend event-ring capacity for one capture.
pub const MAX_EVENT_CAPACITY: usize = 65_536;

/// Largest accepted `returned` count in one query.
pub const MAX_QUERY_LIMIT: usize = 10_000;
