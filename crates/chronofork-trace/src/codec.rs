//! Canonical `CFTRACE1` v1 encoder and reject-only decoder.
//!
//! There is exactly one layout implementation. The decoder validates every
//! declared length against its documented bound and the remaining byte count
//! before it allocates the record vector, so a forged count cannot force a large
//! allocation.

use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{StateHash, Tick};

use crate::error::TraceError;
use crate::meta::{TraceMeta, TraceStopReason};
use crate::record::{InterruptPhase, TraceRecord};
use crate::{
    MAX_BACKEND_ID_BYTES, MAX_BACKEND_VERSION_BYTES, MAX_TRACE_BYTES, MAX_TRACE_CAPACITY,
    MAX_TRACE_RECORDS, TRACE_FORMAT_VERSION, TRACE_MAGIC,
};

/// Smallest possible encoded record: kind byte, index, tick and a two-byte
/// payload (instruction opcode or controller port/value).
const MIN_RECORD_BYTES: usize = 1 + 8 + 8 + 2;

const FLAG_EVENTS_OBSERVED: u16 = 1 << 0;
const FLAG_TRIGGERED: u16 = 1 << 1;
const FLAG_BITS: u16 = FLAG_EVENTS_OBSERVED | FLAG_TRIGGERED;

/// Encodes a validated trace into canonical v1 bytes.
///
/// The `Trace` constructor has already validated every field and length, so the
/// writer is infallible; this function never allocates more than the encoded
/// value needs.
pub(crate) fn encode(meta: &TraceMeta, records: &[TraceRecord]) -> Vec<u8> {
    let mut writer = Writer::with_capacity(
        TRACE_MAGIC.len()
            + 2
            + 2
            + 1
            + 4
            + 4
            + 8
            + 8
            + 8
            + 8
            + 32
            + 2
            + meta.backend.len()
            + 2
            + meta.backend_version.len()
            + records.len() * 26,
    );
    writer.write_bytes(TRACE_MAGIC);
    writer.write_u16_be(TRACE_FORMAT_VERSION);
    let mut flags = 0_u16;
    if meta.events_observed {
        flags |= FLAG_EVENTS_OBSERVED;
    }
    if meta.triggered {
        flags |= FLAG_TRIGGERED;
    }
    writer.write_u16_be(flags);
    writer.write_u8(meta.stop_reason.code());
    // The constructor bounds the record count and capacity well below u32 and
    // the string byte lengths well below u16, so these conversions are exact.
    writer.write_u32_be(records.len() as u32);
    writer.write_u32_be(meta.capacity as u32);
    writer.write_u64_be(meta.first_index);
    writer.write_u64_be(meta.dropped);
    writer.write_u64_be(meta.start_tick.raw());
    writer.write_u64_be(meta.end_tick.raw());
    writer.write_bytes(meta.state_hash.as_bytes());
    writer.write_u16_be(meta.backend.len() as u16);
    writer.write_bytes(meta.backend.as_bytes());
    writer.write_u16_be(meta.backend_version.len() as u16);
    writer.write_bytes(meta.backend_version.as_bytes());
    for record in records {
        writer.write_u8(record.kind().code());
        writer.write_u64_be(record.index());
        writer.write_u64_be(record.tick().raw());
        match record {
            TraceRecord::Instruction { pc, opcode, .. } => {
                writer.write_u32_be(*pc);
                writer.write_u16_be(*opcode);
            }
            TraceRecord::BusWrite {
                address,
                value,
                width,
                ..
            } => {
                writer.write_u32_be(*address);
                writer.write_u32_be(*value);
                writer.write_u8(*width);
            }
            TraceRecord::VdpPort {
                offset,
                value,
                width,
                write,
                ..
            } => {
                writer.write_u8(*offset);
                writer.write_u32_be(*value);
                writer.write_u8(*width);
                writer.write_u8(u8::from(*write));
            }
            TraceRecord::ControllerRead { port, value, .. } => {
                writer.write_u8(*port);
                writer.write_u8(*value);
            }
            TraceRecord::Interrupt { level, phase, .. } => {
                writer.write_u8(*level);
                writer.write_u8(phase.code());
            }
        }
    }
    writer.into_bytes()
}

/// Decodes canonical v1 bytes into metadata and records.
///
/// The decoder rejects every malformed class with a typed error. A forged
/// record count is checked against [`MIN_RECORD_BYTES`] and the remaining byte
/// count before the record vector is allocated.
pub(crate) fn decode(bytes: &[u8]) -> Result<(TraceMeta, Vec<TraceRecord>), TraceError> {
    if bytes.len() > MAX_TRACE_BYTES {
        return Err(TraceError::TooLarge {
            limit: MAX_TRACE_BYTES,
            found: bytes.len(),
        });
    }
    let mut reader = Reader::new(bytes);
    if reader.remaining() < TRACE_MAGIC.len() {
        return Err(TraceError::TruncatedInput {
            needed: TRACE_MAGIC.len(),
            offset: 0,
            remaining: reader.remaining(),
        });
    }
    let magic = reader.read_exact(TRACE_MAGIC.len()).map_err(codec_error)?;
    if magic != TRACE_MAGIC {
        return Err(TraceError::InvalidMagic);
    }
    let version = read_u16(&mut reader)?;
    if version != TRACE_FORMAT_VERSION {
        return Err(TraceError::UnsupportedVersion { found: version });
    }
    let flags = read_u16(&mut reader)?;
    if flags & !FLAG_BITS != 0 {
        return Err(TraceError::InvalidMetadata(format!(
            "trace flags 0b{flags:016b} set reserved bits"
        )));
    }
    let stop_reason_code = read_u8(&mut reader)?;
    let stop_reason = TraceStopReason::from_code(stop_reason_code).ok_or_else(|| {
        TraceError::InvalidMetadata(format!(
            "trace stop reason {stop_reason_code} is outside 0..=2"
        ))
    })?;
    let record_count =
        usize::try_from(read_u32(&mut reader)?).map_err(|_| TraceError::InvalidRecordCount {
            count: usize::MAX,
            capacity: 0,
        })?;
    let capacity =
        usize::try_from(read_u32(&mut reader)?).map_err(|_| TraceError::InvalidCapacity {
            capacity: usize::MAX,
        })?;
    if capacity == 0 || capacity > MAX_TRACE_CAPACITY {
        return Err(TraceError::InvalidCapacity { capacity });
    }
    if record_count > MAX_TRACE_RECORDS || record_count > capacity {
        return Err(TraceError::InvalidRecordCount {
            count: record_count,
            capacity,
        });
    }
    let first_index = read_u64(&mut reader)?;
    let dropped = read_u64(&mut reader)?;
    if first_index != dropped {
        return Err(TraceError::InvalidMetadata(format!(
            "first_index {first_index} must equal dropped {dropped}"
        )));
    }
    let start_tick = Tick::from_raw(read_u64(&mut reader)?);
    let end_tick = Tick::from_raw(read_u64(&mut reader)?);
    if end_tick < start_tick {
        return Err(TraceError::InvalidMetadata(format!(
            "end_tick {end_tick} is earlier than start_tick {start_tick}"
        )));
    }
    let state_hash = StateHash::from_bytes(read_array_32(&mut reader)?);
    let backend = read_string(
        &mut reader,
        MAX_BACKEND_ID_BYTES,
        "trace backend identifier",
    )?;
    let backend_version = read_string(
        &mut reader,
        MAX_BACKEND_VERSION_BYTES,
        "trace backend version",
    )?;

    // Validate the declared count against the remaining bytes before allocating
    // the record vector. A short body therefore cannot force a large allocation.
    let remaining = reader.remaining();
    let minimum_bytes =
        record_count
            .checked_mul(MIN_RECORD_BYTES)
            .ok_or(TraceError::InvalidRecordCount {
                count: record_count,
                capacity,
            })?;
    if minimum_bytes > remaining {
        return Err(TraceError::TruncatedInput {
            needed: minimum_bytes,
            offset: bytes.len() - remaining,
            remaining,
        });
    }

    let mut records = Vec::with_capacity(record_count);
    let mut expected_index = first_index;
    let mut previous_tick: Option<Tick> = None;
    for _ in 0..record_count {
        let kind_code = read_u8(&mut reader)?;
        let index = read_u64(&mut reader)?;
        let tick = Tick::from_raw(read_u64(&mut reader)?);
        if index != expected_index {
            return Err(TraceError::NonContiguousIndex {
                expected: expected_index,
                found: index,
            });
        }
        if let Some(previous) = previous_tick {
            if tick < previous {
                return Err(TraceError::DecreasingTick {
                    previous: previous.raw(),
                    found: tick.raw(),
                });
            }
        }
        if tick < start_tick || tick > end_tick {
            return Err(TraceError::InvalidRecordField(format!(
                "record tick {tick} is outside the capture window {start_tick}..={end_tick}"
            )));
        }
        previous_tick = Some(tick);
        expected_index = expected_index.checked_add(1).ok_or_else(|| {
            TraceError::InvalidMetadata("record index range overflows u64".to_owned())
        })?;
        let record = match kind_code {
            0 => TraceRecord::Instruction {
                index,
                tick,
                pc: read_u32(&mut reader)?,
                opcode: read_u16(&mut reader)?,
            },
            1 => {
                let address = read_u32(&mut reader)?;
                let value = read_u32(&mut reader)?;
                let width = read_u8(&mut reader)?;
                if !(1..=4).contains(&width) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "bus write width {width} is outside 1..=4"
                    )));
                }
                TraceRecord::BusWrite {
                    index,
                    tick,
                    address,
                    value,
                    width,
                }
            }
            2 => {
                let offset = read_u8(&mut reader)?;
                let value = read_u32(&mut reader)?;
                let width = read_u8(&mut reader)?;
                if !(1..=2).contains(&width) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "VDP port width {width} is outside 1..=2"
                    )));
                }
                let write = match read_u8(&mut reader)? {
                    0 => false,
                    1 => true,
                    other => {
                        return Err(TraceError::InvalidRecordField(format!(
                            "VDP port write flag {other} is outside 0..=1"
                        )));
                    }
                };
                TraceRecord::VdpPort {
                    index,
                    tick,
                    offset,
                    value,
                    width,
                    write,
                }
            }
            3 => {
                let port = read_u8(&mut reader)?;
                if !(1..=2).contains(&port) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "controller port {port} is outside 1..=2"
                    )));
                }
                TraceRecord::ControllerRead {
                    index,
                    tick,
                    port,
                    value: read_u8(&mut reader)?,
                }
            }
            4 => {
                let level = read_u8(&mut reader)?;
                if !(1..=7).contains(&level) {
                    return Err(TraceError::InvalidRecordField(format!(
                        "interrupt level {level} is outside 1..=7"
                    )));
                }
                let phase = match read_u8(&mut reader)? {
                    0 => InterruptPhase::Requested,
                    1 => InterruptPhase::Delivered,
                    2 => InterruptPhase::Acknowledged,
                    other => {
                        return Err(TraceError::InvalidRecordField(format!(
                            "interrupt phase {other} is outside 0..=2"
                        )));
                    }
                };
                TraceRecord::Interrupt {
                    index,
                    tick,
                    level,
                    phase,
                }
            }
            other => return Err(TraceError::UnknownRecordKind { kind: other }),
        };
        records.push(record);
    }
    reader.finish().map_err(codec_error)?;
    let meta = TraceMeta {
        backend,
        backend_version,
        state_hash,
        start_tick,
        end_tick,
        capacity,
        first_index,
        dropped,
        stop_reason,
        triggered: flags & FLAG_TRIGGERED != 0,
        events_observed: flags & FLAG_EVENTS_OBSERVED != 0,
    };
    Ok((meta, records))
}

fn read_u8(reader: &mut Reader<'_>) -> Result<u8, TraceError> {
    reader.read_u8().map_err(codec_error)
}

fn read_u16(reader: &mut Reader<'_>) -> Result<u16, TraceError> {
    reader.read_u16_be().map_err(codec_error)
}

fn read_u32(reader: &mut Reader<'_>) -> Result<u32, TraceError> {
    reader.read_u32_be().map_err(codec_error)
}

fn read_u64(reader: &mut Reader<'_>) -> Result<u64, TraceError> {
    reader.read_u64_be().map_err(codec_error)
}

fn read_array_32(reader: &mut Reader<'_>) -> Result<[u8; 32], TraceError> {
    reader.read_array::<32>().map_err(codec_error)
}

/// Reads a big-endian-`u16`-length-prefixed, non-empty UTF-8 string.
///
/// The byte bound is checked before the body is read and before the owned
/// string is allocated.
fn read_string(
    reader: &mut Reader<'_>,
    max_bytes: usize,
    field: &'static str,
) -> Result<String, TraceError> {
    let length = usize::from(read_u16(reader)?);
    if length == 0 {
        return Err(TraceError::InvalidMetadata(format!(
            "{field} must not be empty"
        )));
    }
    if length > max_bytes {
        return Err(TraceError::InvalidMetadata(format!(
            "{field} is {length} bytes, limit is {max_bytes}"
        )));
    }
    let bytes = reader.read_exact(length).map_err(codec_error)?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| TraceError::InvalidMetadata(format!("{field} is not valid UTF-8")))?;
    Ok(text.to_owned())
}

fn codec_error(error: CodecError) -> TraceError {
    match error {
        CodecError::Truncated {
            needed,
            offset,
            remaining,
        } => TraceError::TruncatedInput {
            needed,
            offset,
            remaining,
        },
        CodecError::LengthOverflow => {
            TraceError::InvalidMetadata("trace length arithmetic overflow".to_owned())
        }
        CodecError::TrailingBytes { count } => TraceError::TrailingBytes { count },
        CodecError::InvalidUtf8(_) => {
            TraceError::InvalidMetadata("trace string is not valid UTF-8".to_owned())
        }
        CodecError::EmptyString => {
            TraceError::InvalidMetadata("trace string must not be empty".to_owned())
        }
    }
}
