//! Query, comparison and one-way TSV export acceptance tests.

use std::error::Error;

use chronofork_core::{StateHash, Tick};
use chronofork_trace::{
    InterruptPhase, MAX_QUERY_LIMIT, Trace, TraceDivergenceReason, TraceError, TraceKind,
    TraceKindMask, TraceMeta, TraceQuery, TraceRecord, TraceStopReason,
};

fn sample_meta() -> TraceMeta {
    TraceMeta {
        backend: "chronofork.test".to_owned(),
        backend_version: "0.1.0".to_owned(),
        state_hash: StateHash::from_bytes([0x22; 32]),
        start_tick: Tick::from_raw(3),
        end_tick: Tick::from_raw(20),
        capacity: 16,
        first_index: 3,
        dropped: 3,
        stop_reason: TraceStopReason::StepLimit,
        triggered: true,
        events_observed: true,
    }
}

fn sample_records() -> Vec<TraceRecord> {
    vec![
        TraceRecord::Instruction {
            index: 3,
            tick: Tick::from_raw(3),
            pc: 0x000200,
            opcode: 0x7001,
        },
        TraceRecord::BusWrite {
            index: 4,
            tick: Tick::from_raw(4),
            address: 0x00FF_0000,
            value: 0x2A,
            width: 1,
        },
        TraceRecord::ControllerRead {
            index: 5,
            tick: Tick::from_raw(5),
            port: 1,
            value: 0x7F,
        },
        TraceRecord::VdpPort {
            index: 6,
            tick: Tick::from_raw(6),
            offset: 4,
            value: 0x8F02,
            width: 2,
            write: true,
        },
        TraceRecord::Interrupt {
            index: 7,
            tick: Tick::from_raw(7),
            level: 4,
            phase: InterruptPhase::Requested,
        },
    ]
}

fn sample_trace() -> Result<Trace, TraceError> {
    Trace::new(sample_meta(), sample_records())
}

fn indexes(records: &[TraceRecord]) -> Vec<u64> {
    records.iter().map(TraceRecord::index).collect()
}

#[test]
fn query_filters_by_tick_kind_pc_and_address() -> Result<(), Box<dyn Error>> {
    let trace = sample_trace()?;

    let all = trace.query(&TraceQuery::new(100))?;
    assert_eq!(all.matched, 5);
    assert_eq!(indexes(&all.returned), vec![3_u64, 4, 5, 6, 7]);
    assert!(!all.truncated);

    // Inclusive tick range.
    let window = trace.query(
        &TraceQuery::new(100)
            .with_from_tick(Tick::from_raw(4))
            .with_to_tick(Tick::from_raw(6)),
    )?;
    assert_eq!(indexes(&window.returned), vec![4_u64, 5, 6]);
    let single = trace.query(
        &TraceQuery::new(100)
            .with_from_tick(Tick::from_raw(5))
            .with_to_tick(Tick::from_raw(5)),
    )?;
    assert_eq!(indexes(&single.returned), vec![5_u64]);
    let empty_window = trace.query(
        &TraceQuery::new(100)
            .with_from_tick(Tick::from_raw(30))
            .with_to_tick(Tick::from_raw(40)),
    )?;
    assert_eq!(empty_window.matched, 0);
    assert!(empty_window.returned.is_empty());
    assert!(!empty_window.truncated);

    // Kind masks.
    let instruction_only = trace.query(
        &TraceQuery::new(100).with_kinds(TraceKindMask::none().with(TraceKind::Instruction)),
    )?;
    assert_eq!(indexes(&instruction_only.returned), vec![3_u64]);
    let device = trace.query(
        &TraceQuery::new(100).with_kinds(
            TraceKindMask::none()
                .with(TraceKind::VdpPort)
                .with(TraceKind::ControllerRead),
        ),
    )?;
    assert_eq!(indexes(&device.returned), vec![5_u64, 6]);
    let no_kinds = trace.query(&TraceQuery::new(100).with_kinds(TraceKindMask::none()))?;
    assert_eq!(no_kinds.matched, 0);

    // `pc` matches only instruction records.
    let pc = trace.query(&TraceQuery::new(100).with_pc(0x000200))?;
    assert_eq!(indexes(&pc.returned), vec![3_u64]);
    let missing_pc = trace.query(&TraceQuery::new(100).with_pc(0x000201))?;
    assert_eq!(missing_pc.matched, 0);

    // Numeric address matches bus-write address, VDP offset and controller port.
    let bus = trace.query(&TraceQuery::new(100).with_address(0x00FF_0000))?;
    assert_eq!(indexes(&bus.returned), vec![4_u64]);
    let vdp = trace.query(&TraceQuery::new(100).with_address(4))?;
    assert_eq!(indexes(&vdp.returned), vec![6_u64]);
    let controller = trace.query(&TraceQuery::new(100).with_address(1))?;
    assert_eq!(indexes(&controller.returned), vec![5_u64]);

    // A numeric address equal to an instruction PC never matches the instruction.
    let instruction_pc_window = trace.query(
        &TraceQuery::new(100)
            .with_kinds(TraceKindMask::none().with(TraceKind::Instruction))
            .with_address(0x000200),
    )?;
    assert_eq!(instruction_pc_window.matched, 0);

    // Repeated queries are identical.
    assert_eq!(
        trace.query(&TraceQuery::new(100).with_from_tick(Tick::from_raw(4)))?,
        trace.query(&TraceQuery::new(100).with_from_tick(Tick::from_raw(4)))?
    );
    Ok(())
}

#[test]
fn query_limit_and_bounds_are_validated() -> Result<(), Box<dyn Error>> {
    let trace = sample_trace()?;
    let limited = trace.query(&TraceQuery::new(2))?;
    assert_eq!(limited.matched, 5);
    assert_eq!(limited.returned.len(), 2);
    assert!(limited.truncated);
    let exact = trace.query(&TraceQuery::new(5))?;
    assert_eq!(exact.returned.len(), 5);
    assert!(!exact.truncated);
    assert_eq!(TraceQuery::new(1).limit, 1);

    for limit in [0, MAX_QUERY_LIMIT + 1] {
        match trace.query(&TraceQuery::new(limit)) {
            Ok(result) => {
                return Err(
                    format!("limit {limit} was accepted with {} matches", result.matched).into(),
                );
            }
            Err(error) => assert_eq!(error.code(), "invalid_query"),
        }
    }
    let inverted = trace.query(
        &TraceQuery::new(10)
            .with_from_tick(Tick::from_raw(9))
            .with_to_tick(Tick::from_raw(8)),
    );
    match inverted {
        Ok(_) => return Err("an inverted tick range was accepted".into()),
        Err(error) => {
            assert_eq!(error.code(), "invalid_query");
            assert!(!error.to_string().contains("/home/"));
        }
    }
    let maximum = trace.query(&TraceQuery::new(MAX_QUERY_LIMIT))?;
    assert_eq!(maximum.returned.len(), 5);
    Ok(())
}

#[test]
fn compare_reports_record_and_length_first_divergence() -> Result<(), Box<dyn Error>> {
    let left = sample_trace()?;
    let identical = left.compare(&left);
    assert!(identical.identical);
    assert!(identical.metadata_equal);
    assert!(identical.divergence.is_none());

    let mut changed_records = sample_records();
    changed_records[1] = TraceRecord::ControllerRead {
        index: 4,
        tick: Tick::from_raw(4),
        port: 2,
        value: 0x7F,
    };
    let changed = Trace::new(sample_meta(), changed_records)?;
    let comparison = left.compare(&changed);
    assert!(!comparison.identical);
    assert!(comparison.metadata_equal);
    let divergence = comparison.divergence.clone().ok_or("record divergence")?;
    assert_eq!(divergence.index, 4);
    assert_eq!(divergence.tick, Tick::from_raw(4));
    assert_eq!(divergence.reason, TraceDivergenceReason::Record);
    assert_eq!(divergence.left, Some(left.records()[1].clone()));
    assert_eq!(divergence.right, Some(changed.records()[1].clone()));

    let shorter = Trace::new(sample_meta(), sample_records()[..4].to_vec())?;
    let comparison = left.compare(&shorter);
    let divergence = comparison.divergence.ok_or("length divergence")?;
    assert_eq!(divergence.index, 7);
    assert_eq!(divergence.reason, TraceDivergenceReason::RightMissing);
    assert!(divergence.left.is_some());
    assert!(divergence.right.is_none());
    assert_eq!(
        divergence.left.as_ref().map(TraceRecord::tick),
        Some(Tick::from_raw(7))
    );

    let comparison = shorter.compare(&left);
    let divergence = comparison.divergence.ok_or("length divergence")?;
    assert_eq!(divergence.index, 7);
    assert_eq!(divergence.reason, TraceDivergenceReason::LeftMissing);
    assert!(divergence.left.is_none());
    assert!(divergence.right.is_some());

    // Comparison walks absolute indexes, not vector positions: windows with a
    // different first retained index diverge at the first missing index.
    let later_meta = TraceMeta {
        first_index: 4,
        dropped: 4,
        ..sample_meta()
    };
    let later = Trace::new(later_meta, sample_records()[1..].to_vec())?;
    let comparison = left.compare(&later);
    let divergence = comparison.divergence.ok_or("window divergence")?;
    assert_eq!(divergence.index, 3);
    assert_eq!(divergence.reason, TraceDivergenceReason::RightMissing);

    // A metadata-only difference never decides record identity.
    let metadata_only = Trace::new(
        TraceMeta {
            state_hash: StateHash::from_bytes([0x44; 32]),
            ..sample_meta()
        },
        sample_records(),
    )?;
    let comparison = left.compare(&metadata_only);
    assert!(comparison.identical);
    assert!(!comparison.metadata_equal);
    assert!(comparison.divergence.is_none());

    // Comparison is deterministic and never re-simulates.
    assert_eq!(left.compare(&changed), left.compare(&changed));
    Ok(())
}

#[test]
fn tsv_export_is_fixed_and_deterministic() -> Result<(), Box<dyn Error>> {
    let meta = TraceMeta {
        backend: "chronofork.test".to_owned(),
        backend_version: "0.1.0".to_owned(),
        state_hash: StateHash::from_bytes([0xAB; 32]),
        start_tick: Tick::from_raw(5),
        end_tick: Tick::from_raw(20),
        capacity: 8,
        first_index: 0,
        dropped: 0,
        stop_reason: TraceStopReason::StepLimit,
        triggered: true,
        events_observed: false,
    };
    let records = vec![
        TraceRecord::Instruction {
            index: 0,
            tick: Tick::from_raw(5),
            pc: 0x000200,
            opcode: 0x7001,
        },
        TraceRecord::BusWrite {
            index: 1,
            tick: Tick::from_raw(6),
            address: 0x00FF_0000,
            value: 0x2A,
            width: 1,
        },
        TraceRecord::VdpPort {
            index: 2,
            tick: Tick::from_raw(7),
            offset: 4,
            value: 0x8F02,
            width: 2,
            write: true,
        },
        TraceRecord::ControllerRead {
            index: 3,
            tick: Tick::from_raw(8),
            port: 1,
            value: 0x7F,
        },
        TraceRecord::Interrupt {
            index: 4,
            tick: Tick::from_raw(9),
            level: 4,
            phase: InterruptPhase::Acknowledged,
        },
    ];
    let trace = Trace::new(meta, records)?;
    let expected = concat!(
        "# chronofork trace v1\n",
        "# backend=chronofork.test\n",
        "# backend_version=0.1.0\n",
        "# state_hash=abababababababababababababababababababababababababababababababab\n",
        "# start_tick=5\n",
        "# end_tick=20\n",
        "# capacity=8\n",
        "# records=5\n",
        "# first_index=0\n",
        "# dropped=0\n",
        "# stop_reason=step_limit\n",
        "# triggered=1\n",
        "# events_observed=0\n",
        "index\tkind\ttick\tpc\taddress\tvalue\twidth\textra\n",
        "0\tinstruction\t5\t000200\t-\t00007001\t-\t-\n",
        "1\tbus_write\t6\t-\t00FF0000\t0000002A\t1\t-\n",
        "2\tvdp_port\t7\t-\t00000004\t00008F02\t2\twrite\n",
        "3\tcontroller_read\t8\t-\t00000001\t0000007F\t-\t-\n",
        "4\tinterrupt\t9\t-\t-\t00000004\t-\tacknowledged\n",
    );
    let exported = trace.to_tsv();
    assert_eq!(exported, expected);
    assert_eq!(trace.to_tsv(), exported, "export must be deterministic");
    assert!(!exported.contains('/'), "TSV must not contain host paths");
    assert!(!exported.contains("\\"));

    // An empty trace still emits the complete comment header and table header.
    let empty_meta = TraceMeta {
        capacity: 4,
        first_index: 0,
        dropped: 0,
        start_tick: Tick::ZERO,
        end_tick: Tick::ZERO,
        ..trace.meta().clone()
    };
    let empty = Trace::new(empty_meta, Vec::new())?;
    let empty_export = empty.to_tsv();
    assert!(empty_export.starts_with("# chronofork trace v1\n# backend=chronofork.test\n"));
    assert!(empty_export.contains("# records=0\n"));
    assert!(empty_export.ends_with("index\tkind\ttick\tpc\taddress\tvalue\twidth\textra\n"));
    assert_eq!(empty_export.lines().count(), 14);

    // There is no TSV import API: the only import is the canonical decoder.
    assert!(Trace::from_bytes(exported.as_bytes()).is_err());
    Ok(())
}
