//! Versioned, portable checkpoint envelopes.
//!
//! The outer envelope is owned by ChronoFork and carries identity, lineage and
//! integrity metadata. The backend payload remains opaque. Encoding is
//! explicitly versioned so a future format change can be rejected instead of
//! being misread.

use std::fmt;

use chronofork_backend_api::BackendSnapshot;
use chronofork_core::{SessionId, SnapshotId, StateHash, hash_bytes};

use crate::error::SessionError;

/// Current outer snapshot format version.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 1;

const MAGIC: [u8; 8] = *b"CFSNAP01";
const CHECKSUM_LEN: usize = 32;
const MIN_HEADER_LEN: usize = MAGIC.len()
    + 2 // format version
    + 8 // snapshot id
    + 8 // origin session id
    + 2 // backend id length
    + 2 // backend version length
    + 2 // backend payload format version
    + 8 // tick
    + 32 // config hash
    + 32 // state hash
    + 8 // payload length
    + CHECKSUM_LEN;

/// A persisted checkpoint produced by a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    id: SnapshotId,
    origin_session: SessionId,
    format_version: u16,
    backend: BackendSnapshot,
}

impl Snapshot {
    /// Creates a snapshot using the current outer format version.
    #[must_use]
    pub fn new(id: SnapshotId, origin_session: SessionId, backend: BackendSnapshot) -> Self {
        Self {
            id,
            origin_session,
            format_version: SNAPSHOT_FORMAT_VERSION,
            backend,
        }
    }

    /// Returns the snapshot identifier.
    #[must_use]
    pub const fn id(&self) -> SnapshotId {
        self.id
    }

    /// Returns the session that produced the snapshot.
    #[must_use]
    pub const fn origin_session(&self) -> SessionId {
        self.origin_session
    }

    /// Returns the outer snapshot format version.
    #[must_use]
    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// Returns the backend-owned state.
    #[must_use]
    pub fn backend(&self) -> &BackendSnapshot {
        &self.backend
    }

    /// Returns the tick recorded by the backend.
    #[must_use]
    pub const fn tick(&self) -> chronofork_core::Tick {
        self.backend.tick()
    }

    /// Returns the state digest recorded by the backend.
    #[must_use]
    pub const fn state_hash(&self) -> StateHash {
        self.backend.state_hash()
    }

    /// Serializes the snapshot into a self-verifying byte envelope.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SessionError> {
        let payload = self.backend.payload();
        let payload_len = u64::try_from(payload.len()).map_err(|_| {
            SessionError::InvalidSnapshot("snapshot payload does not fit in u64".to_owned())
        })?;
        let backend_id = self.backend.backend_id().as_str().as_bytes();
        let backend_version = self.backend.backend_version().as_bytes();
        let backend_id_len = u16::try_from(backend_id.len()).map_err(|_| {
            SessionError::InvalidSnapshot("backend id is too long to serialize".to_owned())
        })?;
        let backend_version_len = u16::try_from(backend_version.len()).map_err(|_| {
            SessionError::InvalidSnapshot("backend version is too long to serialize".to_owned())
        })?;

        let mut output = Vec::with_capacity(MIN_HEADER_LEN + payload.len());
        output.extend_from_slice(&MAGIC);
        output.extend_from_slice(&self.format_version.to_le_bytes());
        output.extend_from_slice(&self.id.raw().to_le_bytes());
        output.extend_from_slice(&self.origin_session.raw().to_le_bytes());
        output.extend_from_slice(&backend_id_len.to_le_bytes());
        output.extend_from_slice(backend_id);
        output.extend_from_slice(&backend_version_len.to_le_bytes());
        output.extend_from_slice(backend_version);
        output.extend_from_slice(&self.backend.format_version().to_le_bytes());
        output.extend_from_slice(&self.backend.tick().raw().to_le_bytes());
        output.extend_from_slice(self.backend.config_hash().as_bytes());
        output.extend_from_slice(self.backend.state_hash().as_bytes());
        output.extend_from_slice(&payload_len.to_le_bytes());
        output.extend_from_slice(payload);
        let checksum = hash_bytes(&output);
        output.extend_from_slice(checksum.as_bytes());
        Ok(output)
    }

    /// Parses and validates a serialized snapshot.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SessionError> {
        if bytes.len() < MIN_HEADER_LEN {
            return Err(SessionError::InvalidSnapshot(format!(
                "snapshot is {} bytes, minimum is {MIN_HEADER_LEN}",
                bytes.len()
            )));
        }
        let body_len = bytes.len() - CHECKSUM_LEN;
        let (body, checksum) = bytes.split_at(body_len);
        if hash_bytes(body).as_bytes() != checksum {
            return Err(SessionError::InvalidSnapshot(
                "snapshot checksum mismatch".to_owned(),
            ));
        }

        let mut reader = Reader::new(body);
        let magic = reader.read_array::<8>()?;
        if magic != MAGIC {
            return Err(SessionError::InvalidSnapshot(
                "snapshot magic mismatch".to_owned(),
            ));
        }
        let format_version = reader.read_u16()?;
        if format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(SessionError::IncompatibleSnapshot(format!(
                "outer format version {format_version}, expected {SNAPSHOT_FORMAT_VERSION}"
            )));
        }
        let snapshot_id = SnapshotId::from_raw(reader.read_u64()?);
        let origin_session = SessionId::from_raw(reader.read_u64()?);
        let backend_id = reader.read_string()?;
        let backend_version = reader.read_string()?;
        let backend_format_version = reader.read_u16()?;
        let tick = chronofork_core::Tick::from_raw(reader.read_u64()?);
        let config_hash = StateHash::from_bytes(reader.read_array::<32>()?);
        let state_hash = StateHash::from_bytes(reader.read_array::<32>()?);
        let payload_len_u64 = reader.read_u64()?;
        let payload_len = usize::try_from(payload_len_u64).map_err(|_| {
            SessionError::InvalidSnapshot("payload length does not fit in usize".to_owned())
        })?;
        let payload = reader.read_exact(payload_len)?.to_vec();
        if reader.remaining() != 0 {
            return Err(SessionError::InvalidSnapshot(format!(
                "snapshot has {} trailing bytes",
                reader.remaining()
            )));
        }

        let backend = BackendSnapshot::new(
            chronofork_backend_api::BackendId::new(backend_id),
            backend_version,
            backend_format_version,
            tick,
            config_hash,
            state_hash,
            payload,
        );
        Ok(Self {
            id: snapshot_id,
            origin_session,
            format_version,
            backend,
        })
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at tick {} ({})",
            self.id,
            self.tick(),
            self.state_hash()
        )
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], SessionError> {
        let end = self.position.checked_add(length).ok_or_else(|| {
            SessionError::InvalidSnapshot("snapshot length arithmetic overflow".to_owned())
        })?;
        if end > self.bytes.len() {
            return Err(SessionError::InvalidSnapshot(format!(
                "snapshot truncated: needed {length} bytes at offset {}, only {} remain",
                self.position,
                self.remaining()
            )));
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], SessionError> {
        let slice = self.read_exact(N)?;
        let mut output = [0_u8; N];
        output.copy_from_slice(slice);
        Ok(output)
    }

    fn read_u16(&mut self) -> Result<u16, SessionError> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    fn read_u64(&mut self) -> Result<u64, SessionError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_string(&mut self) -> Result<String, SessionError> {
        let length = usize::from(self.read_u16()?);
        let bytes = self.read_exact(length)?;
        let text = std::str::from_utf8(bytes).map_err(|error| {
            SessionError::InvalidSnapshot(format!("snapshot string is not UTF-8: {error}"))
        })?;
        if text.is_empty() {
            return Err(SessionError::InvalidSnapshot(
                "snapshot string must not be empty".to_owned(),
            ));
        }
        Ok(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{BackendId, BackendSnapshot};
    use chronofork_core::{SessionId, SnapshotId, StateHash, Tick};

    use super::{SNAPSHOT_FORMAT_VERSION, Snapshot};

    fn sample_snapshot() -> Snapshot {
        let backend = BackendSnapshot::new(
            BackendId::new("chronofork.test.synthetic"),
            "0.1.0",
            1,
            Tick::from_raw(42),
            StateHash::from_bytes([0x11; 32]),
            StateHash::from_bytes([0x22; 32]),
            vec![1, 2, 3, 4],
        );
        Snapshot::new(SnapshotId::from_raw(7), SessionId::from_raw(3), backend)
    }

    #[test]
    fn snapshot_round_trips_through_bytes() {
        let snapshot = sample_snapshot();
        let bytes = snapshot.to_bytes().expect("sample snapshot serializes");
        let decoded = Snapshot::from_bytes(&bytes).expect("sample snapshot decodes");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.format_version(), SNAPSHOT_FORMAT_VERSION);
    }

    #[test]
    fn corruption_is_rejected() {
        let mut bytes = sample_snapshot().to_bytes().expect("sample serializes");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert!(Snapshot::from_bytes(&bytes).is_err());
    }

    #[test]
    fn unsupported_outer_version_is_rejected() {
        let mut bytes = sample_snapshot().to_bytes().expect("sample serializes");
        let version_offset = 8;
        bytes[version_offset..version_offset + 2]
            .copy_from_slice(&(SNAPSHOT_FORMAT_VERSION + 1).to_le_bytes());
        let checksum_offset = bytes.len() - 32;
        let checksum = chronofork_core::hash_bytes(&bytes[..checksum_offset]);
        bytes[checksum_offset..].copy_from_slice(checksum.as_bytes());
        assert!(Snapshot::from_bytes(&bytes).is_err());
    }

    #[test]
    fn truncation_is_rejected() {
        let bytes = sample_snapshot().to_bytes().expect("sample serializes");
        assert!(Snapshot::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }
}
