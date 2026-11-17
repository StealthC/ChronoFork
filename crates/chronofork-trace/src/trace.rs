//! The validated [`Trace`] value and its canonical construction rules.

use chronofork_core::Tick;

use crate::codec;
use crate::error::TraceError;
use crate::meta::{TraceMeta, TraceStopReason};
use crate::record::TraceRecord;
use crate::{
    MAX_BACKEND_ID_BYTES, MAX_BACKEND_VERSION_BYTES, MAX_TRACE_CAPACITY, MAX_TRACE_RECORDS,
};

/// A validated, bounded window of structured trace records.
///
/// Values are produced only by [`Trace::new`], [`Trace::from_bytes`] or
/// [`crate::capture()`]. Every construction path enforces the same invariants:
///
/// - metadata strings are non-empty and within their byte bounds;
/// - `capacity` is in `1..=MAX_TRACE_CAPACITY` and `records.len() <= capacity`;
/// - `first_index == dropped`;
/// - record indexes are contiguous from `first_index`;
/// - record ticks are non-decreasing and inside `start_tick..=end_tick`;
/// - record fields are in their documented ranges.
///
/// A `Trace` is a host observation artifact, never canonical machine state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trace {
    meta: TraceMeta,
    records: Vec<TraceRecord>,
}

impl Trace {
    /// Validates metadata and records into a trace.
    ///
    /// # Errors
    ///
    /// Returns a typed [`TraceError`] for the first violated invariant and never
    /// keeps a partially validated value.
    pub fn new(meta: TraceMeta, records: Vec<TraceRecord>) -> Result<Self, TraceError> {
        validate_meta(&meta)?;
        if records.len() > MAX_TRACE_RECORDS {
            return Err(TraceError::InvalidRecordCount {
                count: records.len(),
                capacity: MAX_TRACE_RECORDS,
            });
        }
        if records.len() > meta.capacity {
            return Err(TraceError::InvalidRecordCount {
                count: records.len(),
                capacity: meta.capacity,
            });
        }
        let mut previous_tick: Option<Tick> = None;
        for (position, record) in records.iter().enumerate() {
            let offset = u64::try_from(position).map_err(|_| {
                TraceError::InvalidMetadata("record position does not fit u64".to_owned())
            })?;
            let expected = meta.first_index.checked_add(offset).ok_or_else(|| {
                TraceError::InvalidMetadata("record index range overflows u64".to_owned())
            })?;
            if record.index() != expected {
                return Err(TraceError::NonContiguousIndex {
                    expected,
                    found: record.index(),
                });
            }
            let tick = record.tick();
            if let Some(previous) = previous_tick {
                if tick < previous {
                    return Err(TraceError::DecreasingTick {
                        previous: previous.raw(),
                        found: tick.raw(),
                    });
                }
            }
            if tick < meta.start_tick || tick > meta.end_tick {
                return Err(TraceError::InvalidRecordField(format!(
                    "record tick {tick} is outside the capture window {}..={}",
                    meta.start_tick, meta.end_tick
                )));
            }
            previous_tick = Some(tick);
            validate_record_fields(record)?;
        }
        Ok(Self { meta, records })
    }

    /// Returns the validated metadata.
    #[must_use]
    pub const fn meta(&self) -> &TraceMeta {
        &self.meta
    }

    /// Returns the retained records in absolute-index order.
    #[must_use]
    pub fn records(&self) -> &[TraceRecord] {
        &self.records
    }

    /// Returns the number of retained records.
    #[must_use]
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Returns why capture stopped.
    #[must_use]
    pub const fn stop_reason(&self) -> TraceStopReason {
        self.meta.stop_reason
    }

    /// Encodes the trace into canonical `CFTRACE1` v1 bytes.
    ///
    /// The encoding is infallible after construction and byte-deterministic:
    /// identical values produce identical bytes and no host data is included.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        codec::encode(&self.meta, &self.records)
    }

    /// Decodes canonical `CFTRACE1` v1 bytes.
    ///
    /// Import is reject-only: there is no migration, best-effort parse or
    /// version coercion. The declared record count is validated against the
    /// remaining byte count before the record vector is allocated.
    ///
    /// # Errors
    ///
    /// Returns a typed [`TraceError`] for the first malformed field; no input is
    /// partially accepted.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TraceError> {
        let (meta, records) = codec::decode(bytes)?;
        Self::new(meta, records)
    }
}

/// Enforces the metadata bounds and relationships documented on [`TraceMeta`].
fn validate_meta(meta: &TraceMeta) -> Result<(), TraceError> {
    if meta.backend.is_empty() {
        return Err(TraceError::InvalidMetadata(
            "backend identifier must not be empty".to_owned(),
        ));
    }
    if meta.backend.len() > MAX_BACKEND_ID_BYTES {
        return Err(TraceError::InvalidMetadata(format!(
            "backend identifier is {} bytes, limit is {MAX_BACKEND_ID_BYTES}",
            meta.backend.len()
        )));
    }
    if meta.backend_version.is_empty() {
        return Err(TraceError::InvalidMetadata(
            "backend version must not be empty".to_owned(),
        ));
    }
    if meta.backend_version.len() > MAX_BACKEND_VERSION_BYTES {
        return Err(TraceError::InvalidMetadata(format!(
            "backend version is {} bytes, limit is {MAX_BACKEND_VERSION_BYTES}",
            meta.backend_version.len()
        )));
    }
    if meta.capacity == 0 || meta.capacity > MAX_TRACE_CAPACITY {
        return Err(TraceError::InvalidCapacity {
            capacity: meta.capacity,
        });
    }
    if meta.first_index != meta.dropped {
        return Err(TraceError::InvalidMetadata(format!(
            "first_index {} must equal dropped {}",
            meta.first_index, meta.dropped
        )));
    }
    if meta.end_tick < meta.start_tick {
        return Err(TraceError::InvalidMetadata(format!(
            "end_tick {} is earlier than start_tick {}",
            meta.end_tick, meta.start_tick
        )));
    }
    Ok(())
}

/// Enforces the documented range of every kind-specific record field.
fn validate_record_fields(record: &TraceRecord) -> Result<(), TraceError> {
    match record {
        TraceRecord::Instruction { .. } => Ok(()),
        TraceRecord::BusWrite { width, .. } => {
            if (1..=4).contains(width) {
                Ok(())
            } else {
                Err(TraceError::InvalidRecordField(format!(
                    "bus write width {width} is outside 1..=4"
                )))
            }
        }
        TraceRecord::VdpPort { width, .. } => {
            if (1..=2).contains(width) {
                Ok(())
            } else {
                Err(TraceError::InvalidRecordField(format!(
                    "VDP port width {width} is outside 1..=2"
                )))
            }
        }
        TraceRecord::ControllerRead { port, .. } => {
            if (1..=2).contains(port) {
                Ok(())
            } else {
                Err(TraceError::InvalidRecordField(format!(
                    "controller port {port} is outside 1..=2"
                )))
            }
        }
        TraceRecord::Interrupt { level, .. } => {
            if (1..=7).contains(level) {
                Ok(())
            } else {
                Err(TraceError::InvalidRecordField(format!(
                    "interrupt level {level} is outside 1..=7"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chronofork_core::{StateHash, Tick};

    use super::Trace;
    use crate::error::TraceError;
    use crate::meta::{TraceMeta, TraceStopReason};
    use crate::record::{InterruptPhase, TraceRecord};
    use crate::{MAX_BACKEND_ID_BYTES, MAX_BACKEND_VERSION_BYTES, MAX_TRACE_CAPACITY};

    fn meta() -> TraceMeta {
        TraceMeta {
            backend: "chronofork.test".to_owned(),
            backend_version: "0.1.0".to_owned(),
            state_hash: StateHash::from_bytes([0x11; 32]),
            start_tick: Tick::from_raw(0),
            end_tick: Tick::from_raw(2),
            capacity: 4,
            first_index: 0,
            dropped: 0,
            stop_reason: TraceStopReason::StepLimit,
            triggered: true,
            events_observed: false,
        }
    }

    fn records() -> Vec<TraceRecord> {
        vec![
            TraceRecord::Instruction {
                index: 0,
                tick: Tick::from_raw(0),
                pc: 0x200,
                opcode: 0x7001,
            },
            TraceRecord::VdpPort {
                index: 1,
                tick: Tick::from_raw(1),
                offset: 4,
                value: 0x8F02,
                width: 2,
                write: true,
            },
        ]
    }

    #[test]
    fn valid_boundaries_are_accepted() {
        let boundary_meta = TraceMeta {
            backend: "b".repeat(MAX_BACKEND_ID_BYTES),
            backend_version: "v".repeat(MAX_BACKEND_VERSION_BYTES),
            capacity: MAX_TRACE_CAPACITY,
            end_tick: Tick::from_raw(0),
            ..meta()
        };
        assert!(Trace::new(boundary_meta, Vec::new()).is_ok());

        let minimum = TraceMeta {
            backend: "b".to_owned(),
            backend_version: "v".to_owned(),
            capacity: 1,
            ..meta()
        };
        assert!(Trace::new(minimum, Vec::new()).is_ok());
    }

    #[test]
    fn metadata_invariants_are_rejected() {
        let empty_backend = TraceMeta {
            backend: String::new(),
            ..meta()
        };
        assert!(matches!(
            Trace::new(empty_backend, Vec::new()),
            Err(TraceError::InvalidMetadata(_))
        ));

        let oversized_backend = TraceMeta {
            backend: "b".repeat(MAX_BACKEND_ID_BYTES + 1),
            ..meta()
        };
        assert!(matches!(
            Trace::new(oversized_backend, Vec::new()),
            Err(TraceError::InvalidMetadata(_))
        ));

        let empty_version = TraceMeta {
            backend_version: String::new(),
            ..meta()
        };
        assert!(matches!(
            Trace::new(empty_version, Vec::new()),
            Err(TraceError::InvalidMetadata(_))
        ));

        let zero_capacity = TraceMeta {
            capacity: 0,
            ..meta()
        };
        assert_eq!(
            Trace::new(zero_capacity, Vec::new()),
            Err(TraceError::InvalidCapacity { capacity: 0 })
        );

        let oversized_capacity = TraceMeta {
            capacity: MAX_TRACE_CAPACITY + 1,
            ..meta()
        };
        assert_eq!(
            Trace::new(oversized_capacity, Vec::new()),
            Err(TraceError::InvalidCapacity {
                capacity: MAX_TRACE_CAPACITY + 1
            })
        );

        let mismatched_index = TraceMeta {
            first_index: 2,
            dropped: 1,
            ..meta()
        };
        assert!(matches!(
            Trace::new(mismatched_index, Vec::new()),
            Err(TraceError::InvalidMetadata(_))
        ));

        let reversed_ticks = TraceMeta {
            start_tick: Tick::from_raw(2),
            end_tick: Tick::from_raw(1),
            ..meta()
        };
        assert!(matches!(
            Trace::new(reversed_ticks, Vec::new()),
            Err(TraceError::InvalidMetadata(_))
        ));
    }

    #[test]
    fn record_invariants_are_rejected() {
        let too_many_for_capacity = TraceMeta {
            capacity: 1,
            ..meta()
        };
        assert_eq!(
            Trace::new(too_many_for_capacity, records()),
            Err(TraceError::InvalidRecordCount {
                count: 2,
                capacity: 1
            })
        );

        let mut wrong_index = records();
        wrong_index[1] = TraceRecord::ControllerRead {
            index: 5,
            tick: Tick::from_raw(1),
            port: 1,
            value: 0x7F,
        };
        assert_eq!(
            Trace::new(meta(), wrong_index),
            Err(TraceError::NonContiguousIndex {
                expected: 1,
                found: 5
            })
        );

        let decreasing = vec![
            TraceRecord::Instruction {
                index: 0,
                tick: Tick::from_raw(1),
                pc: 0x200,
                opcode: 0x7001,
            },
            TraceRecord::ControllerRead {
                index: 1,
                tick: Tick::from_raw(0),
                port: 1,
                value: 0x7F,
            },
        ];
        assert_eq!(
            Trace::new(meta(), decreasing),
            Err(TraceError::DecreasingTick {
                previous: 1,
                found: 0
            })
        );

        let mut out_of_window = records();
        out_of_window[1] = TraceRecord::ControllerRead {
            index: 1,
            tick: Tick::from_raw(9),
            port: 1,
            value: 0x7F,
        };
        assert!(matches!(
            Trace::new(meta(), out_of_window),
            Err(TraceError::InvalidRecordField(_))
        ));

        let mut bad_width = records();
        bad_width[1] = TraceRecord::VdpPort {
            index: 1,
            tick: Tick::from_raw(1),
            offset: 4,
            value: 0,
            width: 3,
            write: true,
        };
        assert!(matches!(
            Trace::new(meta(), bad_width),
            Err(TraceError::InvalidRecordField(_))
        ));

        let mut bad_port = records();
        bad_port[1] = TraceRecord::ControllerRead {
            index: 1,
            tick: Tick::from_raw(1),
            port: 3,
            value: 0,
        };
        assert!(matches!(
            Trace::new(meta(), bad_port),
            Err(TraceError::InvalidRecordField(_))
        ));

        let mut bad_level = records();
        bad_level[1] = TraceRecord::Interrupt {
            index: 1,
            tick: Tick::from_raw(1),
            level: 8,
            phase: InterruptPhase::Requested,
        };
        assert!(matches!(
            Trace::new(meta(), bad_level),
            Err(TraceError::InvalidRecordField(_))
        ));
    }

    #[test]
    fn accessors_expose_the_validated_values() {
        let trace = Trace::new(meta(), records()).expect("valid trace");
        assert_eq!(trace.record_count(), 2);
        assert_eq!(trace.records(), records());
        assert_eq!(trace.meta(), &meta());
        assert_eq!(trace.stop_reason(), TraceStopReason::StepLimit);
    }
}
