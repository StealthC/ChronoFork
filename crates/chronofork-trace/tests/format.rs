//! Canonical `CFTRACE1` v1 codec acceptance tests.
//!
//! These tests exercise the single codec implementation: round-trip, byte
//! determinism, every malformed class from ADR-0011 §3.2.3 and the construction
//! bounds of [`Trace::new`]. They return `Result` instead of unwrapping so the
//! workspace `expect_used` lint stays satisfied without a file-level allowance.

use std::error::Error;

use chronofork_core::{StateHash, Tick};
use chronofork_trace::{
    InterruptPhase, MAX_BACKEND_ID_BYTES, MAX_BACKEND_VERSION_BYTES, MAX_TRACE_BYTES,
    MAX_TRACE_CAPACITY, MAX_TRACE_RECORDS, TRACE_MAGIC, Trace, TraceError, TraceMeta, TraceRecord,
    TraceStopReason,
};
use proptest::prelude::*;

const OFF_VERSION: usize = 8;
const OFF_FLAGS: usize = 10;
const OFF_STOP: usize = 12;
const OFF_COUNT: usize = 13;
const OFF_CAPACITY: usize = 17;
const OFF_DROPPED: usize = 29;
const OFF_END_TICK: usize = 45;
const OFF_HASH: usize = 53;
const OFF_BACKEND_LEN: usize = 85;
const HEADER_FIXED: usize = 85;

fn sample_meta() -> TraceMeta {
    TraceMeta {
        backend: "b".repeat(MAX_BACKEND_ID_BYTES),
        backend_version: "v".repeat(MAX_BACKEND_VERSION_BYTES),
        state_hash: StateHash::from_bytes([0x5A; 32]),
        start_tick: Tick::from_raw(4),
        end_tick: Tick::from_raw(20),
        capacity: 16,
        first_index: 7,
        dropped: 7,
        stop_reason: TraceStopReason::StopCondition,
        triggered: true,
        events_observed: true,
    }
}

fn sample_records() -> Vec<TraceRecord> {
    vec![
        TraceRecord::Instruction {
            index: 7,
            tick: Tick::from_raw(10),
            pc: 0x000200,
            opcode: 0x7001,
        },
        TraceRecord::BusWrite {
            index: 8,
            tick: Tick::from_raw(11),
            address: 0x00FF_0000,
            value: 0x2A,
            width: 1,
        },
        TraceRecord::VdpPort {
            index: 9,
            tick: Tick::from_raw(12),
            offset: 4,
            value: 0x8F02,
            width: 2,
            write: true,
        },
        TraceRecord::ControllerRead {
            index: 10,
            tick: Tick::from_raw(13),
            port: 1,
            value: 0x7F,
        },
        TraceRecord::Interrupt {
            index: 11,
            tick: Tick::from_raw(14),
            level: 4,
            phase: InterruptPhase::Requested,
        },
    ]
}

fn sample_trace() -> Result<Trace, TraceError> {
    Trace::new(sample_meta(), sample_records())
}

fn header_len(backend: &str, version: &str) -> usize {
    HEADER_FIXED + 2 + backend.len() + 2 + version.len()
}

fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn set_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

fn single_trace(record: TraceRecord) -> Result<Vec<u8>, TraceError> {
    let meta = TraceMeta {
        backend: "chronofork.test".to_owned(),
        backend_version: "0.1.0".to_owned(),
        state_hash: StateHash::from_bytes([0x11; 32]),
        start_tick: Tick::from_raw(0),
        end_tick: Tick::from_raw(20),
        capacity: 4,
        first_index: 0,
        dropped: 0,
        stop_reason: TraceStopReason::StepLimit,
        triggered: true,
        events_observed: false,
    };
    Ok(Trace::new(meta, vec![record])?.to_bytes())
}

fn expect_error(bytes: &[u8], expected_code: &str) -> Result<(), Box<dyn Error>> {
    match Trace::from_bytes(bytes) {
        Ok(trace) => Err(format!(
            "malformed input was accepted ({} records, {} bytes)",
            trace.record_count(),
            bytes.len()
        )
        .into()),
        Err(error) => {
            if error.code() != expected_code {
                return Err(format!(
                    "expected code {expected_code}, got {} ({error})",
                    error.code()
                )
                .into());
            }
            if error.to_string().is_empty() {
                return Err("trace error display text must not be empty".into());
            }
            if error.to_string().contains("/home/") || error.to_string().contains("\\") {
                return Err(format!("error text leaked a host path: {error}").into());
            }
            Ok(())
        }
    }
}

#[test]
fn round_trip_preserves_all_record_kinds_and_metadata() -> Result<(), Box<dyn Error>> {
    let trace = sample_trace()?;
    assert!(trace.record_count() == 5);
    assert_eq!(trace.stop_reason(), TraceStopReason::StopCondition);
    assert!(trace.meta().triggered);
    assert!(trace.meta().events_observed);
    assert_eq!(trace.meta().first_index, trace.meta().dropped);
    assert_eq!(trace.records()[0].index(), 7);

    let bytes = trace.to_bytes();
    assert_eq!(&bytes[..TRACE_MAGIC.len()], TRACE_MAGIC, "magic prefix");
    let decoded = Trace::from_bytes(&bytes)?;
    assert_eq!(decoded, trace);
    assert_eq!(decoded.meta(), trace.meta());
    assert_eq!(decoded.records(), trace.records());
    assert_eq!(decoded.to_bytes(), bytes, "re-encoding is byte-identical");

    let expected_summaries = [
        "instruction index=7 tick=10 pc=000200 opcode=7001",
        "bus_write index=8 tick=11 address=00FF0000 value=0000002A width=1",
        "vdp_port index=9 tick=12 offset=04 value=00008F02 width=2 write=1",
        "controller_read index=10 tick=13 port=1 value=7F",
        "interrupt index=11 tick=14 level=4 phase=requested",
    ];
    for (record, expected) in decoded.records().iter().zip(expected_summaries) {
        assert_eq!(record.summary(), expected);
    }
    Ok(())
}

#[test]
fn encoding_is_byte_deterministic() -> Result<(), Box<dyn Error>> {
    let first = sample_trace()?;
    let second = sample_trace()?;
    let first_bytes = first.to_bytes();
    let second_bytes = second.to_bytes();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_bytes, first.to_bytes());
    assert_eq!(
        first_bytes,
        Trace::from_bytes(&first_bytes)?.to_bytes(),
        "decode/re-encode must be stable"
    );

    // The header is big-endian and carries the documented fixed fields.
    assert_eq!(
        u16::from_be_bytes([first_bytes[OFF_VERSION], first_bytes[OFF_VERSION + 1]]),
        1
    );
    assert_eq!(
        u16::from_be_bytes([first_bytes[OFF_FLAGS], first_bytes[OFF_FLAGS + 1]]),
        0b11
    );
    assert_eq!(first_bytes[OFF_STOP], 2);
    assert_eq!(
        u32::from_be_bytes([
            first_bytes[OFF_COUNT],
            first_bytes[OFF_COUNT + 1],
            first_bytes[OFF_COUNT + 2],
            first_bytes[OFF_COUNT + 3],
        ]),
        5
    );
    assert_eq!(
        u32::from_be_bytes([
            first_bytes[OFF_CAPACITY],
            first_bytes[OFF_CAPACITY + 1],
            first_bytes[OFF_CAPACITY + 2],
            first_bytes[OFF_CAPACITY + 3],
        ]),
        16
    );
    assert_eq!(
        &first_bytes[OFF_HASH..OFF_HASH + 32],
        StateHash::from_bytes([0x5A; 32]).as_bytes()
    );
    let backend_len = usize::from(u16::from_be_bytes([
        first_bytes[OFF_BACKEND_LEN],
        first_bytes[OFF_BACKEND_LEN + 1],
    ]));
    assert_eq!(backend_len, MAX_BACKEND_ID_BYTES);
    assert_eq!(
        first_bytes.len(),
        header_len(
            &"b".repeat(MAX_BACKEND_ID_BYTES),
            &"v".repeat(MAX_BACKEND_VERSION_BYTES)
        ) + 23
            + 26
            + 24
            + 19
            + 19
    );
    Ok(())
}

#[test]
fn rejects_malformed_canonical_input() -> Result<(), Box<dyn Error>> {
    let valid = sample_trace()?.to_bytes();
    let records = header_len(&sample_meta().backend, &sample_meta().backend_version);
    let second_record = records + 23;

    // Wrong magic and every unsupported version.
    let mut bytes = valid.clone();
    bytes[0] = b'X';
    expect_error(&bytes, "invalid_magic")?;
    for version in [0_u16, 2, u16::MAX] {
        let mut bytes = valid.clone();
        set_u16(&mut bytes, OFF_VERSION, version);
        expect_error(&bytes, "unsupported_version")?;
    }

    // Reserved flag bits and an invalid stop reason.
    let mut bytes = valid.clone();
    set_u16(&mut bytes, OFF_FLAGS, 0b100);
    expect_error(&bytes, "invalid_metadata")?;
    let mut bytes = valid.clone();
    bytes[OFF_STOP] = 3;
    expect_error(&bytes, "invalid_metadata")?;

    // Truncation at every granularity.
    expect_error(&valid[..4], "truncated_input")?;
    expect_error(&valid[..OFF_HASH], "truncated_input")?;
    expect_error(&valid[..valid.len() - 1], "truncated_input")?;
    let cut = valid.len() - MAX_BACKEND_VERSION_BYTES;
    expect_error(&valid[..cut], "truncated_input")?;

    // Trailing bytes and invalid metadata relationships.
    let mut bytes = valid.clone();
    bytes.push(0);
    expect_error(&bytes, "trailing_bytes")?;
    let mut bytes = valid.clone();
    set_u64(&mut bytes, OFF_END_TICK, 3);
    expect_error(&bytes, "invalid_metadata")?;
    let mut bytes = valid.clone();
    set_u64(&mut bytes, OFF_DROPPED, 8);
    expect_error(&bytes, "invalid_metadata")?;

    // Capacity and record-count bounds.
    let mut bytes = valid.clone();
    set_u32(&mut bytes, OFF_CAPACITY, 0);
    expect_error(&bytes, "invalid_capacity")?;
    let mut bytes = valid.clone();
    set_u32(
        &mut bytes,
        OFF_CAPACITY,
        u32::try_from(MAX_TRACE_CAPACITY + 1)?,
    );
    expect_error(&bytes, "invalid_capacity")?;
    let mut bytes = valid.clone();
    set_u32(&mut bytes, OFF_COUNT, 17);
    expect_error(&bytes, "invalid_record_count")?;
    let mut bytes = valid.clone();
    set_u32(&mut bytes, OFF_COUNT, u32::try_from(MAX_TRACE_RECORDS + 1)?);
    set_u32(&mut bytes, OFF_CAPACITY, u32::try_from(MAX_TRACE_CAPACITY)?);
    expect_error(&bytes, "invalid_record_count")?;

    // A forged huge count over a short body must fail before a large allocation.
    let mut bytes = valid.clone();
    set_u32(&mut bytes, OFF_COUNT, u32::try_from(MAX_TRACE_RECORDS)?);
    set_u32(&mut bytes, OFF_CAPACITY, u32::try_from(MAX_TRACE_CAPACITY)?);
    expect_error(&bytes, "truncated_input")?;
    let mut bytes = valid.clone();
    set_u32(&mut bytes, OFF_COUNT, 10);
    expect_error(&bytes, "truncated_input")?;

    // Backend strings: empty, oversized, non-UTF-8 and truncated.
    let mut bytes = valid.clone();
    set_u16(&mut bytes, OFF_BACKEND_LEN, 0);
    expect_error(&bytes, "invalid_metadata")?;
    let mut bytes = valid.clone();
    set_u16(
        &mut bytes,
        OFF_BACKEND_LEN,
        u16::try_from(MAX_BACKEND_ID_BYTES + 1)?,
    );
    expect_error(&bytes, "invalid_metadata")?;
    let mut bytes = valid.clone();
    bytes[OFF_BACKEND_LEN + 2] = 0xFF;
    expect_error(&bytes, "invalid_metadata")?;

    // Record-level rejections.
    let mut bytes = valid.clone();
    bytes[records] = 5;
    expect_error(&bytes, "unknown_record_kind")?;
    let mut bytes = valid.clone();
    bytes[records] = 255;
    expect_error(&bytes, "unknown_record_kind")?;
    let mut bytes = valid.clone();
    set_u64(&mut bytes, records + 1, 9);
    expect_error(&bytes, "non_contiguous_index")?;
    let mut bytes = valid.clone();
    set_u64(&mut bytes, second_record + 9, 9);
    expect_error(&bytes, "decreasing_tick")?;
    let mut bytes = valid.clone();
    set_u64(&mut bytes, records + 9, 25);
    expect_error(&bytes, "invalid_record_field")?;

    // Invalid kind-specific fields.
    let bus = single_trace(TraceRecord::BusWrite {
        index: 0,
        tick: Tick::from_raw(0),
        address: 0x200,
        value: 1,
        width: 1,
    })?;
    let bus_payload = header_len("chronofork.test", "0.1.0") + 17;
    for width in [0_u8, 5] {
        let mut bytes = bus.clone();
        bytes[bus_payload + 8] = width;
        expect_error(&bytes, "invalid_record_field")?;
    }

    let vdp = single_trace(TraceRecord::VdpPort {
        index: 0,
        tick: Tick::from_raw(0),
        offset: 4,
        value: 0x8F02,
        width: 2,
        write: true,
    })?;
    for width in [0_u8, 3] {
        let mut bytes = vdp.clone();
        bytes[bus_payload + 5] = width;
        expect_error(&bytes, "invalid_record_field")?;
    }
    let mut bytes = vdp.clone();
    bytes[bus_payload + 6] = 2;
    expect_error(&bytes, "invalid_record_field")?;

    let controller = single_trace(TraceRecord::ControllerRead {
        index: 0,
        tick: Tick::from_raw(0),
        port: 1,
        value: 0x7F,
    })?;
    for port in [0_u8, 3] {
        let mut bytes = controller.clone();
        bytes[bus_payload] = port;
        expect_error(&bytes, "invalid_record_field")?;
    }

    let interrupt = single_trace(TraceRecord::Interrupt {
        index: 0,
        tick: Tick::from_raw(0),
        level: 4,
        phase: InterruptPhase::Delivered,
    })?;
    for level in [0_u8, 8] {
        let mut bytes = interrupt.clone();
        bytes[bus_payload] = level;
        expect_error(&bytes, "invalid_record_field")?;
    }
    let mut bytes = interrupt.clone();
    bytes[bus_payload + 1] = 3;
    expect_error(&bytes, "invalid_record_field")?;

    // A body above the encoded-size limit is rejected before parsing.
    let oversized = vec![0_u8; MAX_TRACE_BYTES + 1];
    expect_error(&oversized, "too_large")?;
    Ok(())
}

#[test]
fn trace_construction_bounds_are_enforced() -> Result<(), Box<dyn Error>> {
    let boundary = TraceMeta {
        backend: "b".repeat(MAX_BACKEND_ID_BYTES),
        backend_version: "v".repeat(MAX_BACKEND_VERSION_BYTES),
        capacity: MAX_TRACE_CAPACITY,
        start_tick: Tick::from_raw(9),
        end_tick: Tick::from_raw(9),
        first_index: 0,
        dropped: 0,
        ..sample_meta()
    };
    assert!(Trace::new(boundary, Vec::new()).is_ok());

    let minimum = TraceMeta {
        backend: "b".to_owned(),
        backend_version: "v".to_owned(),
        capacity: 1,
        first_index: 0,
        dropped: 0,
        ..sample_meta()
    };
    assert!(Trace::new(minimum.clone(), Vec::new()).is_ok());

    let cases: Vec<(TraceMeta, &str)> = vec![
        (
            TraceMeta {
                backend: String::new(),
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
        (
            TraceMeta {
                backend: "b".repeat(MAX_BACKEND_ID_BYTES + 1),
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
        (
            TraceMeta {
                backend_version: String::new(),
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
        (
            TraceMeta {
                backend_version: "v".repeat(MAX_BACKEND_VERSION_BYTES + 1),
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
        (
            TraceMeta {
                capacity: 0,
                ..minimum.clone()
            },
            "invalid_capacity",
        ),
        (
            TraceMeta {
                capacity: MAX_TRACE_CAPACITY + 1,
                ..minimum.clone()
            },
            "invalid_capacity",
        ),
        (
            TraceMeta {
                end_tick: Tick::from_raw(8),
                start_tick: Tick::from_raw(9),
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
        (
            TraceMeta {
                first_index: 2,
                dropped: 1,
                ..minimum.clone()
            },
            "invalid_metadata",
        ),
    ];
    for (meta, code) in cases {
        match Trace::new(meta, Vec::new()) {
            Ok(_) => return Err(format!("invalid metadata was accepted with code {code}").into()),
            Err(error) => {
                assert_eq!(error.code(), code);
                assert!(!error.to_string().contains("/home/"));
            }
        }
    }

    let too_many = vec![
        TraceRecord::Instruction {
            index: 0,
            tick: Tick::from_raw(0),
            pc: 0,
            opcode: 0,
        },
        TraceRecord::Instruction {
            index: 1,
            tick: Tick::from_raw(1),
            pc: 2,
            opcode: 0,
        },
    ];
    assert_eq!(
        Trace::new(minimum.clone(), too_many).map(|_| ()),
        Err(TraceError::InvalidRecordCount {
            count: 2,
            capacity: 1
        })
    );

    // Every kind-specific boundary value is accepted and each out-of-range value
    // is a typed field error.
    let accepted = vec![
        TraceRecord::BusWrite {
            index: 0,
            tick: Tick::from_raw(0),
            address: 0,
            value: 0,
            width: 4,
        },
        TraceRecord::VdpPort {
            index: 1,
            tick: Tick::from_raw(1),
            offset: 0,
            value: 0,
            width: 1,
            write: false,
        },
        TraceRecord::ControllerRead {
            index: 2,
            tick: Tick::from_raw(2),
            port: 2,
            value: 0,
        },
        TraceRecord::Interrupt {
            index: 3,
            tick: Tick::from_raw(3),
            level: 7,
            phase: InterruptPhase::Acknowledged,
        },
    ];
    let meta = TraceMeta {
        capacity: 4,
        start_tick: Tick::ZERO,
        end_tick: Tick::from_raw(20),
        ..minimum
    };
    assert!(Trace::new(meta.clone(), accepted).is_ok());

    let rejected = vec![
        TraceRecord::BusWrite {
            index: 0,
            tick: Tick::from_raw(0),
            address: 0,
            value: 0,
            width: 0,
        },
        TraceRecord::VdpPort {
            index: 0,
            tick: Tick::from_raw(0),
            offset: 0,
            value: 0,
            width: 3,
            write: false,
        },
        TraceRecord::ControllerRead {
            index: 0,
            tick: Tick::from_raw(0),
            port: 3,
            value: 0,
        },
        TraceRecord::Interrupt {
            index: 0,
            tick: Tick::from_raw(0),
            level: 8,
            phase: InterruptPhase::Delivered,
        },
    ];
    for record in rejected {
        assert!(
            matches!(
                Trace::new(meta.clone(), vec![record]),
                Err(TraceError::InvalidRecordField(_))
            ),
            "out-of-range fields must be typed record errors"
        );
    }
    Ok(())
}

fn arb_record(kind: u8, index: u64, tick: Tick) -> TraceRecord {
    match kind {
        0 => TraceRecord::Instruction {
            index,
            tick,
            pc: u32::try_from(index).unwrap_or(u32::MAX),
            opcode: 0x4E71,
        },
        1 => TraceRecord::BusWrite {
            index,
            tick,
            address: u32::try_from(index).unwrap_or(u32::MAX),
            value: 0x2A,
            width: 1,
        },
        2 => TraceRecord::VdpPort {
            index,
            tick,
            offset: 4,
            value: 0x8F02,
            width: 2,
            write: true,
        },
        3 => TraceRecord::ControllerRead {
            index,
            tick,
            port: 1,
            value: 0x7F,
        },
        _ => TraceRecord::Interrupt {
            index,
            tick,
            level: 4,
            phase: InterruptPhase::Requested,
        },
    }
}

proptest! {
    #[test]
    fn proptest_round_trip_preserves_trace(
        backend_len in 1_usize..=MAX_BACKEND_ID_BYTES,
        version_len in 1_usize..=MAX_BACKEND_VERSION_BYTES,
        capacity in 1_usize..=64,
        first_index in 0_u64..=100,
        start_tick in 0_u64..=50,
        span in 0_u64..=100,
        kind_codes in proptest::collection::vec(0_u8..=4, 0..=64),
        offsets in proptest::collection::vec(0_u64..=100, 0..=64),
    ) {
        let mut ticks: Vec<u64> = offsets.iter().map(|offset| start_tick + offset % (span + 1)).collect();
        ticks.sort_unstable();
        let count = kind_codes.len().min(ticks.len()).min(capacity);
        let records: Vec<TraceRecord> = (0..count)
            .map(|position| {
                let index = first_index + position as u64;
                arb_record(kind_codes[position], index, Tick::from_raw(ticks[position]))
            })
            .collect();
        let meta = TraceMeta {
            backend: "b".repeat(backend_len),
            backend_version: "v".repeat(version_len),
            state_hash: StateHash::from_bytes([0x33; 32]),
            start_tick: Tick::from_raw(start_tick),
            end_tick: Tick::from_raw(start_tick + span),
            capacity,
            first_index,
            dropped: first_index,
            stop_reason: TraceStopReason::StepLimit,
            triggered: position_is_true(first_index),
            events_observed: capacity % 2 == 0,
        };
        let trace = match Trace::new(meta, records) {
            Ok(trace) => trace,
            Err(error) => {
                return Err(TestCaseError::fail(format!("valid trace rejected: {error}")));
            }
        };
        let bytes = trace.to_bytes();
        let decoded = match Trace::from_bytes(&bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                return Err(TestCaseError::fail(format!("round trip failed: {error}")));
            }
        };
        prop_assert_eq!(&decoded, &trace);
        prop_assert_eq!(decoded.to_bytes(), bytes);
    }
}

fn position_is_true(first_index: u64) -> bool {
    first_index % 2 == 0
}
