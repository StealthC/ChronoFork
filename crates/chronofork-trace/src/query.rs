//! Structured queries and first-divergence comparison.

use chronofork_core::Tick;

use crate::MAX_QUERY_LIMIT;
use crate::error::TraceError;
use crate::meta::TraceMeta;
use crate::record::{TraceKindMask, TraceRecord};
use crate::trace::Trace;

/// Filters applied to a trace's retained window.
///
/// Filters compose with logical AND. `from_tick` and `to_tick` are inclusive;
/// `pc` matches only instruction records and `address` matches the numeric bus
/// write address, VDP port offset or controller port. Use
/// [`TraceQuery::new`] plus the builder methods, or set the public fields
/// directly; [`Trace::query`] validates the combination before scanning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceQuery {
    /// Inclusive lower tick bound.
    pub from_tick: Option<Tick>,
    /// Inclusive upper tick bound.
    pub to_tick: Option<Tick>,
    /// Enabled record kinds; [`TraceKindMask::all`] by default.
    pub kinds: TraceKindMask,
    /// Instruction program counter filter.
    pub pc: Option<u32>,
    /// Numeric address filter for bus writes, VDP ports and controller reads.
    pub address: Option<u32>,
    /// Maximum number of records to return, `1..=`[`crate::MAX_QUERY_LIMIT`].
    pub limit: usize,
}

impl TraceQuery {
    /// Creates a query with every kind enabled and no tick/address filters.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            from_tick: None,
            to_tick: None,
            kinds: TraceKindMask::all(),
            pc: None,
            address: None,
            limit,
        }
    }

    /// Sets the inclusive lower tick bound.
    #[must_use]
    pub fn with_from_tick(mut self, tick: Tick) -> Self {
        self.from_tick = Some(tick);
        self
    }

    /// Sets the inclusive upper tick bound.
    #[must_use]
    pub fn with_to_tick(mut self, tick: Tick) -> Self {
        self.to_tick = Some(tick);
        self
    }

    /// Sets the enabled kind mask.
    #[must_use]
    pub fn with_kinds(mut self, kinds: TraceKindMask) -> Self {
        self.kinds = kinds;
        self
    }

    /// Sets the instruction program counter filter.
    #[must_use]
    pub fn with_pc(mut self, pc: u32) -> Self {
        self.pc = Some(pc);
        self
    }

    /// Sets the numeric address filter.
    #[must_use]
    pub fn with_address(mut self, address: u32) -> Self {
        self.address = Some(address);
        self
    }
}

/// The result of one structured query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceQueryResult {
    /// Number of records matching every filter, before the `limit` cap.
    pub matched: u64,
    /// Matching records in absolute-index order, capped by `limit`.
    pub returned: Vec<TraceRecord>,
    /// Whether `matched` is greater than the returned count.
    pub truncated: bool,
}

/// Why the two traces first differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceDivergenceReason {
    /// Both sides retain a record at the index and their values differ.
    Record,
    /// The left trace retains no record at the index.
    LeftMissing,
    /// The right trace retains no record at the index.
    RightMissing,
}

impl TraceDivergenceReason {
    /// Returns the stable lowercase reason name used by CLI output.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::LeftMissing => "left_missing",
            Self::RightMissing => "right_missing",
        }
    }
}

/// The first record divergence between two traces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceDivergence {
    /// Absolute record index of the divergence.
    pub index: u64,
    /// Tick reported for the divergence, taken from the present record.
    pub tick: Tick,
    /// Left record at the index, or `None` when it is missing.
    pub left: Option<TraceRecord>,
    /// Right record at the index, or `None` when it is missing.
    pub right: Option<TraceRecord>,
    /// Why the index diverged.
    pub reason: TraceDivergenceReason,
}

/// The result of comparing two traces by the union of their retained windows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceComparison {
    /// Whether every index in the union has the same retained record.
    ///
    /// This is record identity only; metadata equality is reported separately.
    pub identical: bool,
    /// Whether the two metadata values are equal.
    pub metadata_equal: bool,
    /// First divergence in ascending absolute-index order, if any.
    pub divergence: Option<TraceDivergence>,
}

impl Trace {
    /// Applies a structured query to the retained records.
    ///
    /// `matched` counts every matching record while `returned` is capped at
    /// `limit`; `truncated` is `matched > returned.len()`. Records are returned
    /// in absolute-index order and the scan never re-simulates.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::InvalidQuery`] when `limit` is outside
    /// `1..=MAX_QUERY_LIMIT` or `from_tick` is later than `to_tick`.
    pub fn query(&self, query: &TraceQuery) -> Result<TraceQueryResult, TraceError> {
        if query.limit == 0 || query.limit > MAX_QUERY_LIMIT {
            return Err(TraceError::InvalidQuery(format!(
                "query limit {} is outside 1..={MAX_QUERY_LIMIT}",
                query.limit
            )));
        }
        if let (Some(from), Some(to)) = (query.from_tick, query.to_tick) {
            if from > to {
                return Err(TraceError::InvalidQuery(format!(
                    "query from_tick {from} is later than to_tick {to}"
                )));
            }
        }
        let mut matched = 0_u64;
        let mut returned = Vec::new();
        for record in self.records() {
            if !record_matches(record, query) {
                continue;
            }
            matched += 1;
            if returned.len() < query.limit {
                returned.push(record.clone());
            }
        }
        let truncated = matched > returned.len() as u64;
        Ok(TraceQueryResult {
            matched,
            returned,
            truncated,
        })
    }

    /// Compares two traces by the union of their retained absolute-index
    /// windows.
    ///
    /// The walk visits every index that either side retains plus the boundary
    /// where one side's window ends, so extra retained records are a divergence
    /// at the first extra index. The first difference wins; metadata equality is
    /// reported but never decides record identity.
    #[must_use]
    pub fn compare(&self, other: &Self) -> TraceComparison {
        let divergence =
            first_divergence(self.meta(), self.records(), other.meta(), other.records());
        TraceComparison {
            identical: divergence.is_none(),
            metadata_equal: self.meta() == other.meta(),
            divergence,
        }
    }
}

fn record_matches(record: &TraceRecord, query: &TraceQuery) -> bool {
    if !query.kinds.contains(record.kind()) {
        return false;
    }
    let tick = record.tick();
    if let Some(from) = query.from_tick {
        if tick < from {
            return false;
        }
    }
    if let Some(to) = query.to_tick {
        if tick > to {
            return false;
        }
    }
    if let Some(pc) = query.pc {
        match record {
            TraceRecord::Instruction { pc: value, .. } if *value == pc => {}
            _ => return false,
        }
    }
    if let Some(address) = query.address {
        let matches = match record {
            TraceRecord::BusWrite { address: value, .. } => *value == address,
            TraceRecord::VdpPort { offset, .. } => u32::from(*offset) == address,
            TraceRecord::ControllerRead { port, .. } => u32::from(*port) == address,
            TraceRecord::Instruction { .. } | TraceRecord::Interrupt { .. } => false,
        };
        if !matches {
            return false;
        }
    }
    true
}

/// Returns the record at an absolute index inside the retained window.
fn record_at<'a>(
    meta: &TraceMeta,
    records: &'a [TraceRecord],
    index: u64,
) -> Option<&'a TraceRecord> {
    let offset = index.checked_sub(meta.first_index)?;
    let position = usize::try_from(offset).ok()?;
    records.get(position)
}

/// Returns the next index at or after `index` that a retained window contains,
/// or `u64::MAX` when neither the current index nor a later one is retained.
fn next_retained_index(meta: &TraceMeta, records: &[TraceRecord], index: u64) -> u64 {
    let following = index.saturating_add(1);
    if records.is_empty() {
        return u64::MAX;
    }
    let end = meta.first_index.saturating_add(records.len() as u64);
    if index < meta.first_index {
        meta.first_index
    } else if following < end {
        following
    } else {
        u64::MAX
    }
}

fn first_divergence(
    left_meta: &TraceMeta,
    left_records: &[TraceRecord],
    right_meta: &TraceMeta,
    right_records: &[TraceRecord],
) -> Option<TraceDivergence> {
    let mut index = match (left_records.is_empty(), right_records.is_empty()) {
        (true, true) => return None,
        (true, false) => right_meta.first_index,
        (false, true) => left_meta.first_index,
        (false, false) => left_meta.first_index.min(right_meta.first_index),
    };
    loop {
        let left = record_at(left_meta, left_records, index);
        let right = record_at(right_meta, right_records, index);
        if left != right {
            let (tick, reason) = match (left, right) {
                (Some(left), Some(_)) => (left.tick(), TraceDivergenceReason::Record),
                (None, Some(right)) => (right.tick(), TraceDivergenceReason::LeftMissing),
                (Some(left), None) => (left.tick(), TraceDivergenceReason::RightMissing),
                (None, None) => return None,
            };
            return Some(TraceDivergence {
                index,
                tick,
                left: left.cloned(),
                right: right.cloned(),
                reason,
            });
        }
        let next = next_retained_index(left_meta, left_records, index).min(next_retained_index(
            right_meta,
            right_records,
            index,
        ));
        if next == u64::MAX {
            return None;
        }
        index = next;
    }
}

#[cfg(test)]
mod tests {
    use chronofork_core::{StateHash, Tick};

    use super::{TraceComparison, TraceDivergenceReason, TraceQuery};
    use crate::error::TraceError;
    use crate::meta::{TraceMeta, TraceStopReason};
    use crate::record::{TraceKind, TraceKindMask, TraceRecord};
    use crate::trace::Trace;

    fn meta(capacity: usize, first_index: u64, end_tick: u64) -> TraceMeta {
        TraceMeta {
            backend: "chronofork.test".to_owned(),
            backend_version: "0.1.0".to_owned(),
            state_hash: StateHash::from_bytes([0x22; 32]),
            start_tick: Tick::ZERO,
            end_tick: Tick::from_raw(end_tick),
            capacity,
            first_index,
            dropped: first_index,
            stop_reason: TraceStopReason::StepLimit,
            triggered: true,
            events_observed: true,
        }
    }

    fn instruction(index: u64, tick: u64, pc: u32, opcode: u16) -> TraceRecord {
        TraceRecord::Instruction {
            index,
            tick: Tick::from_raw(tick),
            pc,
            opcode,
        }
    }

    fn bus_write(index: u64, tick: u64, address: u32, width: u8) -> TraceRecord {
        TraceRecord::BusWrite {
            index,
            tick: Tick::from_raw(tick),
            address,
            value: 0x2A,
            width,
        }
    }

    fn sample() -> Trace {
        let records = vec![
            instruction(0, 0, 0x200, 0x7001),
            bus_write(1, 1, 0x00FF_0000, 1),
            TraceRecord::ControllerRead {
                index: 2,
                tick: Tick::from_raw(2),
                port: 1,
                value: 0x7F,
            },
            TraceRecord::VdpPort {
                index: 3,
                tick: Tick::from_raw(3),
                offset: 4,
                value: 0x8F02,
                width: 2,
                write: true,
            },
            TraceRecord::Interrupt {
                index: 4,
                tick: Tick::from_raw(4),
                level: 4,
                phase: crate::record::InterruptPhase::Requested,
            },
        ];
        Trace::new(meta(8, 0, 4), records).expect("valid sample")
    }

    #[test]
    fn query_filters_compose_inclusively() {
        let trace = sample();
        let all = trace.query(&TraceQuery::new(10)).expect("valid query");
        assert_eq!(all.matched, 5);
        assert_eq!(all.returned.len(), 5);
        assert!(!all.truncated);

        let window = trace
            .query(
                &TraceQuery::new(10)
                    .with_from_tick(Tick::from_raw(1))
                    .with_to_tick(Tick::from_raw(3)),
            )
            .expect("valid query");
        assert_eq!(window.matched, 3);
        assert_eq!(window.returned.first().map(TraceRecord::index), Some(1));
        assert_eq!(window.returned.last().map(TraceRecord::index), Some(3));

        let kinds = TraceKindMask::none()
            .with(TraceKind::Instruction)
            .with(TraceKind::Interrupt);
        let filtered = trace
            .query(&TraceQuery::new(10).with_kinds(kinds))
            .expect("valid query");
        assert_eq!(filtered.matched, 2);
        assert_eq!(
            filtered
                .returned
                .iter()
                .map(TraceRecord::kind)
                .collect::<Vec<_>>(),
            vec![TraceKind::Instruction, TraceKind::Interrupt]
        );

        let pc = trace
            .query(&TraceQuery::new(10).with_pc(0x200))
            .expect("valid query");
        assert_eq!(pc.matched, 1);
        assert_eq!(pc.returned[0].index(), 0);

        let address = trace
            .query(&TraceQuery::new(10).with_address(0x00FF_0000))
            .expect("valid query");
        assert_eq!(address.matched, 1);
        assert_eq!(address.returned[0].kind(), TraceKind::BusWrite);

        let port = trace
            .query(&TraceQuery::new(10).with_address(1))
            .expect("valid query");
        assert_eq!(port.matched, 1);
        assert_eq!(port.returned[0].kind(), TraceKind::ControllerRead);

        let vdp = trace
            .query(&TraceQuery::new(10).with_address(4))
            .expect("valid query");
        assert_eq!(vdp.matched, 1);
        assert_eq!(vdp.returned[0].kind(), TraceKind::VdpPort);

        let none = trace
            .query(&TraceQuery::new(10).with_kinds(TraceKindMask::none()))
            .expect("valid query");
        assert_eq!(none.matched, 0);
        assert!(none.returned.is_empty());
        assert!(!none.truncated);
    }

    #[test]
    fn query_limit_truncates_without_dropping_the_count() {
        let trace = sample();
        let limited = trace.query(&TraceQuery::new(2)).expect("valid query");
        assert_eq!(limited.matched, 5);
        assert_eq!(limited.returned.len(), 2);
        assert!(limited.truncated);
    }

    #[test]
    fn query_bounds_are_validated() {
        let trace = sample();
        assert!(matches!(
            trace.query(&TraceQuery::new(0)),
            Err(TraceError::InvalidQuery(_))
        ));
        assert!(matches!(
            trace.query(&TraceQuery::new(crate::MAX_QUERY_LIMIT + 1)),
            Err(TraceError::InvalidQuery(_))
        ));
        assert!(matches!(
            trace.query(
                &TraceQuery::new(10)
                    .with_from_tick(Tick::from_raw(3))
                    .with_to_tick(Tick::from_raw(2))
            ),
            Err(TraceError::InvalidQuery(_))
        ));
    }

    #[test]
    fn comparison_reports_record_and_length_divergence() {
        let base = sample();
        assert_eq!(
            base.compare(&base),
            TraceComparison {
                identical: true,
                metadata_equal: true,
                divergence: None,
            }
        );

        let mut changed_records = base.records().to_vec();
        changed_records[2] = TraceRecord::ControllerRead {
            index: 2,
            tick: Tick::from_raw(2),
            port: 2,
            value: 0x7F,
        };
        let changed = Trace::new(meta(8, 0, 4), changed_records).expect("valid changed trace");
        let comparison = base.compare(&changed);
        assert!(!comparison.identical);
        assert!(comparison.metadata_equal);
        let divergence = comparison.divergence.expect("record divergence");
        assert_eq!(divergence.index, 2);
        assert_eq!(divergence.tick, Tick::from_raw(2));
        assert_eq!(divergence.reason, TraceDivergenceReason::Record);
        assert_eq!(divergence.left, Some(base.records()[2].clone()));
        assert_eq!(divergence.right, Some(changed.records()[2].clone()));

        let shorter =
            Trace::new(meta(8, 0, 4), base.records()[..4].to_vec()).expect("valid shorter trace");
        let comparison = base.compare(&shorter);
        let divergence = comparison.divergence.expect("length divergence");
        assert_eq!(divergence.index, 4);
        assert_eq!(divergence.reason, TraceDivergenceReason::RightMissing);
        assert!(divergence.left.is_some());
        assert!(divergence.right.is_none());

        let comparison = shorter.compare(&base);
        let divergence = comparison.divergence.expect("length divergence");
        assert_eq!(divergence.reason, TraceDivergenceReason::LeftMissing);
        assert!(divergence.left.is_none());
        assert!(divergence.right.is_some());
    }

    #[test]
    fn comparison_uses_absolute_indexes_and_ignores_metadata_only_changes() {
        let left =
            Trace::new(meta(8, 3, 6), vec![instruction(3, 3, 0x200, 0x7001)]).expect("valid left");
        let same_records =
            Trace::new(meta(8, 3, 6), vec![instruction(3, 3, 0x200, 0x7001)]).expect("valid right");
        let metadata_only = Trace::new(
            TraceMeta {
                state_hash: StateHash::from_bytes([0x33; 32]),
                ..meta(8, 3, 6)
            },
            vec![instruction(3, 3, 0x200, 0x7001)],
        )
        .expect("valid metadata-only trace");
        assert!(left.compare(&same_records).identical);
        let comparison = left.compare(&metadata_only);
        assert!(comparison.identical);
        assert!(!comparison.metadata_equal);

        let later_window = Trace::new(meta(8, 4, 6), vec![instruction(4, 4, 0x202, 0x4E71)])
            .expect("valid later window");
        let comparison = left.compare(&later_window);
        let divergence = comparison.divergence.expect("window divergence");
        assert_eq!(divergence.index, 3);
        assert_eq!(divergence.reason, TraceDivergenceReason::RightMissing);
        assert!(divergence.left.is_some());
        assert!(divergence.right.is_none());

        let earlier_window = Trace::new(meta(8, 2, 6), vec![instruction(2, 2, 0x1FE, 0x4E71)])
            .expect("valid earlier window");
        let comparison = left.compare(&earlier_window);
        let divergence = comparison.divergence.expect("window divergence");
        assert_eq!(divergence.index, 2);
        assert_eq!(divergence.reason, TraceDivergenceReason::LeftMissing);
        assert!(divergence.left.is_none());
        assert!(divergence.right.is_some());
    }
}
