//! Versioned storage metadata records.
//!
//! One record format is used for both record kinds. It is a storage-level record
//! built on [`chronofork_core::codec`]; it is not a snapshot format and never
//! contains backend state bytes. The record has its own magic, format version and
//! SHA-256 checksum, and decoding rejects trailing bytes.

use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{StateHash, Tick, hash_bytes};

use crate::error::SessionError;
use crate::snapshot::Snapshot;
use crate::storage::{STORAGE_METADATA_FORMAT_VERSION, SessionEntry, SnapshotEntry};

pub(crate) const METADATA_MAGIC: [u8; 8] = *b"CFMETA01";
const METADATA_CHECKSUM_LEN: usize = 32;

/// Smallest possible session record: fixed fields with every optional field absent.
const MIN_METADATA_LEN: usize = METADATA_MAGIC.len()
    + 2 // metadata version
    + 1 // record kind
    + 8 // storage session key
    + 8 // recorded session id
    + 1 // name flag
    + 1 // parent session key flag
    + 1 // origin snapshot key flag
    + 8 // next snapshot key
    + 8 // next creation sequence
    + 1 // created-at flag
    + METADATA_CHECKSUM_LEN;

/// Which kind of object a metadata record describes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecordKind {
    Session,
    Snapshot,
}

impl RecordKind {
    const fn code(self) -> u8 {
        match self {
            Self::Session => 1,
            Self::Snapshot => 2,
        }
    }
}

/// Metadata for a durable session directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionMetadata {
    pub(crate) storage_session_key: u64,
    pub(crate) recorded_session_id: u64,
    pub(crate) name: Option<String>,
    pub(crate) parent_session_key: Option<u64>,
    pub(crate) origin_snapshot_key: Option<u64>,
    pub(crate) next_snapshot_key: u64,
    pub(crate) next_creation_sequence: u64,
    pub(crate) created_at_unix_ms: Option<u64>,
}

impl SessionMetadata {
    pub(crate) fn entry(&self) -> SessionEntry {
        SessionEntry {
            session_key: self.storage_session_key,
            name: self.name.clone(),
            parent_session_key: self.parent_session_key,
            origin_snapshot_key: self.origin_snapshot_key,
            next_snapshot_key: self.next_snapshot_key,
            next_creation_sequence: self.next_creation_sequence,
            created_at_unix_ms: self.created_at_unix_ms,
        }
    }

    pub(crate) fn validate_for(&self, session_key: u64) -> Result<(), SessionError> {
        if self.storage_session_key != session_key {
            return Err(SessionError::IdentityMismatch(format!(
                "session metadata key {} does not match directory key {session_key}",
                self.storage_session_key
            )));
        }
        if self.next_snapshot_key == 0 {
            return Err(SessionError::CorruptMetadata(
                "session next_snapshot_key must be at least one".to_owned(),
            ));
        }
        if self.next_creation_sequence == 0 {
            return Err(SessionError::CorruptMetadata(
                "session next_creation_sequence must be at least one".to_owned(),
            ));
        }
        Ok(())
    }

    fn encode_body(&self, writer: &mut Writer) -> Result<(), SessionError> {
        writer.write_u64_le(self.storage_session_key);
        writer.write_u64_le(self.recorded_session_id);
        write_optional_string(writer, self.name.as_deref())?;
        write_optional_u64(writer, self.parent_session_key);
        write_optional_u64(writer, self.origin_snapshot_key);
        writer.write_u64_le(self.next_snapshot_key);
        writer.write_u64_le(self.next_creation_sequence);
        write_optional_u64(writer, self.created_at_unix_ms);
        Ok(())
    }

    fn decode_body(reader: &mut Reader<'_>, max_name_bytes: usize) -> Result<Self, SessionError> {
        let storage_session_key = read_u64(reader)?;
        let recorded_session_id = read_u64(reader)?;
        let name = read_optional_string(reader, max_name_bytes)?;
        let parent_session_key = read_optional_u64(reader)?;
        let origin_snapshot_key = read_optional_u64(reader)?;
        let next_snapshot_key = read_u64(reader)?;
        let next_creation_sequence = read_u64(reader)?;
        let created_at_unix_ms = read_optional_u64(reader)?;
        let metadata = Self {
            storage_session_key,
            recorded_session_id,
            name,
            parent_session_key,
            origin_snapshot_key,
            next_snapshot_key,
            next_creation_sequence,
            created_at_unix_ms,
        };
        validate_durable_keys(&metadata)?;
        Ok(metadata)
    }
}

/// Metadata for a durable snapshot directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotMetadata {
    pub(crate) storage_session_key: u64,
    pub(crate) storage_snapshot_key: u64,
    pub(crate) recorded_snapshot_id: u64,
    pub(crate) recorded_session_id: u64,
    pub(crate) name: Option<String>,
    pub(crate) backend_id: String,
    pub(crate) backend_version: String,
    pub(crate) backend_format_version: u16,
    pub(crate) tick: u64,
    pub(crate) config_hash: [u8; 32],
    pub(crate) state_hash: [u8; 32],
    pub(crate) checkpoint_length: u64,
    pub(crate) checkpoint_sha256: [u8; 32],
    pub(crate) parent_session_key: Option<u64>,
    pub(crate) origin_snapshot_key: Option<u64>,
    pub(crate) creation_sequence: u64,
    pub(crate) created_at_unix_ms: Option<u64>,
}

impl SnapshotMetadata {
    pub(crate) fn entry(&self) -> SnapshotEntry {
        SnapshotEntry {
            session_key: self.storage_session_key,
            snapshot_key: self.storage_snapshot_key,
            name: self.name.clone(),
            creation_sequence: self.creation_sequence,
            tick: Tick::from_raw(self.tick),
            state_hash: StateHash::from_bytes(self.state_hash),
            parent_session_key: self.parent_session_key,
            origin_snapshot_key: self.origin_snapshot_key,
            checkpoint_length: self.checkpoint_length,
            checkpoint_sha256: StateHash::from_bytes(self.checkpoint_sha256),
            created_at_unix_ms: self.created_at_unix_ms,
        }
    }

    pub(crate) fn validate_for(
        &self,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<(), SessionError> {
        if self.storage_session_key != session_key {
            return Err(SessionError::IdentityMismatch(format!(
                "snapshot metadata session key {} does not match directory session key {session_key}",
                self.storage_session_key
            )));
        }
        if self.storage_snapshot_key != snapshot_key {
            return Err(SessionError::IdentityMismatch(format!(
                "snapshot metadata key {} does not match directory key {snapshot_key}",
                self.storage_snapshot_key
            )));
        }
        if self.recorded_snapshot_id != snapshot_key {
            return Err(SessionError::IdentityMismatch(format!(
                "recorded snapshot id {} does not match directory key {snapshot_key}",
                self.recorded_snapshot_id
            )));
        }
        if self.creation_sequence == 0 {
            return Err(SessionError::CorruptMetadata(
                "snapshot creation_sequence must be at least one".to_owned(),
            ));
        }
        if self.checkpoint_length == 0 {
            return Err(SessionError::CorruptMetadata(
                "snapshot checkpoint_length must be at least one".to_owned(),
            ));
        }
        validate_durable_keys(self)?;
        Ok(())
    }

    /// Verifies identity binding and the mirrored envelope fields.
    ///
    /// Compatibility is intentionally not decided here; callers restore through
    /// [`Snapshot::from_bytes`] and the backend `validate_envelope` path.
    pub(crate) fn validate_against(
        &self,
        snapshot: &Snapshot,
        session_key: u64,
        snapshot_key: u64,
    ) -> Result<(), SessionError> {
        self.validate_for(session_key, snapshot_key)?;
        if snapshot.id().raw() != snapshot_key {
            return Err(SessionError::IdentityMismatch(format!(
                "envelope snapshot id {} does not match directory key {snapshot_key}",
                snapshot.id().raw()
            )));
        }
        if self.recorded_session_id != snapshot.origin_session().raw() {
            return Err(SessionError::IdentityMismatch(format!(
                "recorded session id {} does not match envelope origin session {}",
                self.recorded_session_id,
                snapshot.origin_session().raw()
            )));
        }

        let backend = snapshot.backend();
        if self.backend_id != backend.backend_id().as_str() {
            return Err(SessionError::MetadataMismatch(format!(
                "metadata backend_id {} does not match envelope {}",
                self.backend_id,
                backend.backend_id()
            )));
        }
        if self.backend_version != backend.backend_version() {
            return Err(SessionError::MetadataMismatch(format!(
                "metadata backend_version {} does not match envelope {}",
                self.backend_version,
                backend.backend_version()
            )));
        }
        if self.backend_format_version != backend.format_version() {
            return Err(SessionError::MetadataMismatch(format!(
                "metadata backend_format_version {} does not match envelope {}",
                self.backend_format_version,
                backend.format_version()
            )));
        }
        if self.tick != snapshot.tick().raw() {
            return Err(SessionError::MetadataMismatch(format!(
                "metadata tick {} does not match envelope {}",
                self.tick,
                snapshot.tick().raw()
            )));
        }
        if self.config_hash != *backend.config_hash().as_bytes() {
            return Err(SessionError::MetadataMismatch(
                "metadata config_hash does not match envelope".to_owned(),
            ));
        }
        if self.state_hash != *backend.state_hash().as_bytes() {
            return Err(SessionError::MetadataMismatch(
                "metadata state_hash does not match envelope".to_owned(),
            ));
        }
        Ok(())
    }

    fn encode_body(&self, writer: &mut Writer) -> Result<(), SessionError> {
        writer.write_u64_le(self.storage_session_key);
        writer.write_u64_le(self.storage_snapshot_key);
        writer.write_u64_le(self.recorded_snapshot_id);
        writer.write_u64_le(self.recorded_session_id);
        write_optional_string(writer, self.name.as_deref())?;
        write_string(writer, &self.backend_id, "backend id")?;
        write_string(writer, &self.backend_version, "backend version")?;
        writer.write_u16_le(self.backend_format_version);
        writer.write_u64_le(self.tick);
        writer.write_bytes(&self.config_hash);
        writer.write_bytes(&self.state_hash);
        writer.write_u64_le(self.checkpoint_length);
        writer.write_bytes(&self.checkpoint_sha256);
        write_optional_u64(writer, self.parent_session_key);
        write_optional_u64(writer, self.origin_snapshot_key);
        writer.write_u64_le(self.creation_sequence);
        write_optional_u64(writer, self.created_at_unix_ms);
        Ok(())
    }

    fn decode_body(reader: &mut Reader<'_>, max_name_bytes: usize) -> Result<Self, SessionError> {
        let storage_session_key = read_u64(reader)?;
        let storage_snapshot_key = read_u64(reader)?;
        let recorded_snapshot_id = read_u64(reader)?;
        let recorded_session_id = read_u64(reader)?;
        let name = read_optional_string(reader, max_name_bytes)?;
        let backend_id = read_string(reader)?;
        let backend_version = read_string(reader)?;
        let backend_format_version = reader
            .read_u16_le()
            .map_err(|error| codec_metadata(&error))?;
        let tick = read_u64(reader)?;
        let config_hash = reader
            .read_array::<32>()
            .map_err(|error| codec_metadata(&error))?;
        let state_hash = reader
            .read_array::<32>()
            .map_err(|error| codec_metadata(&error))?;
        let checkpoint_length = read_u64(reader)?;
        let checkpoint_sha256 = reader
            .read_array::<32>()
            .map_err(|error| codec_metadata(&error))?;
        let parent_session_key = read_optional_u64(reader)?;
        let origin_snapshot_key = read_optional_u64(reader)?;
        let creation_sequence = read_u64(reader)?;
        let created_at_unix_ms = read_optional_u64(reader)?;
        let metadata = Self {
            storage_session_key,
            storage_snapshot_key,
            recorded_snapshot_id,
            recorded_session_id,
            name,
            backend_id,
            backend_version,
            backend_format_version,
            tick,
            config_hash,
            state_hash,
            checkpoint_length,
            checkpoint_sha256,
            parent_session_key,
            origin_snapshot_key,
            creation_sequence,
            created_at_unix_ms,
        };
        validate_durable_keys(&metadata)?;
        Ok(metadata)
    }
}

/// One decoded storage metadata record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MetadataRecord {
    Session(SessionMetadata),
    Snapshot(SnapshotMetadata),
}

impl MetadataRecord {
    pub(crate) const fn kind(&self) -> RecordKind {
        match self {
            Self::Session(_) => RecordKind::Session,
            Self::Snapshot(_) => RecordKind::Snapshot,
        }
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SessionError> {
        let mut writer = Writer::with_capacity(MIN_METADATA_LEN);
        writer.write_bytes(&METADATA_MAGIC);
        writer.write_u16_le(STORAGE_METADATA_FORMAT_VERSION);
        writer.write_u8(self.kind().code());
        match self {
            Self::Session(metadata) => metadata.encode_body(&mut writer)?,
            Self::Snapshot(metadata) => metadata.encode_body(&mut writer)?,
        }
        let checksum = hash_bytes(writer.as_slice());
        writer.write_bytes(checksum.as_bytes());
        Ok(writer.into_bytes())
    }

    pub(crate) fn decode(bytes: &[u8], max_name_bytes: usize) -> Result<Self, SessionError> {
        if bytes.len() < MIN_METADATA_LEN {
            return Err(SessionError::CorruptMetadata(format!(
                "metadata record is {} bytes, minimum is {MIN_METADATA_LEN}",
                bytes.len()
            )));
        }
        let body_len = bytes.len() - METADATA_CHECKSUM_LEN;
        let (body, checksum) = bytes.split_at(body_len);
        if hash_bytes(body).as_bytes() != checksum {
            return Err(SessionError::CorruptMetadata(
                "metadata checksum mismatch".to_owned(),
            ));
        }

        let mut reader = Reader::new(body);
        let magic = reader
            .read_array::<8>()
            .map_err(|error| codec_metadata(&error))?;
        if magic != METADATA_MAGIC {
            return Err(SessionError::CorruptMetadata(
                "metadata magic mismatch".to_owned(),
            ));
        }
        let version = reader
            .read_u16_le()
            .map_err(|error| codec_metadata(&error))?;
        if version != STORAGE_METADATA_FORMAT_VERSION {
            return Err(SessionError::UnsupportedMetadataVersion {
                found: version,
                supported: STORAGE_METADATA_FORMAT_VERSION,
            });
        }
        let kind = reader.read_u8().map_err(|error| codec_metadata(&error))?;
        let record = match kind {
            1 => Self::Session(SessionMetadata::decode_body(&mut reader, max_name_bytes)?),
            2 => Self::Snapshot(SnapshotMetadata::decode_body(&mut reader, max_name_bytes)?),
            other => {
                return Err(SessionError::CorruptMetadata(format!(
                    "unknown metadata record kind {other}"
                )));
            }
        };
        reader.finish().map_err(|error| {
            SessionError::CorruptMetadata(format!("metadata trailing bytes: {error}"))
        })?;
        Ok(record)
    }
}

fn validate_durable_keys(metadata: &impl DurableKeys) -> Result<(), SessionError> {
    if metadata.storage_session_key() == 0 {
        return Err(SessionError::CorruptMetadata(
            "storage session key must be at least one".to_owned(),
        ));
    }
    if metadata.parent_session_key() == Some(0) {
        return Err(SessionError::CorruptMetadata(
            "parent session key must be at least one".to_owned(),
        ));
    }
    if metadata.origin_snapshot_key() == Some(0) {
        return Err(SessionError::CorruptMetadata(
            "origin snapshot key must be at least one".to_owned(),
        ));
    }
    Ok(())
}

trait DurableKeys {
    fn storage_session_key(&self) -> u64;
    fn parent_session_key(&self) -> Option<u64>;
    fn origin_snapshot_key(&self) -> Option<u64>;
}

impl DurableKeys for SessionMetadata {
    fn storage_session_key(&self) -> u64 {
        self.storage_session_key
    }

    fn parent_session_key(&self) -> Option<u64> {
        self.parent_session_key
    }

    fn origin_snapshot_key(&self) -> Option<u64> {
        self.origin_snapshot_key
    }
}

impl DurableKeys for SnapshotMetadata {
    fn storage_session_key(&self) -> u64 {
        self.storage_session_key
    }

    fn parent_session_key(&self) -> Option<u64> {
        self.parent_session_key
    }

    fn origin_snapshot_key(&self) -> Option<u64> {
        self.origin_snapshot_key
    }
}

fn write_optional_string(writer: &mut Writer, value: Option<&str>) -> Result<(), SessionError> {
    match value {
        Some(value) => {
            writer.write_u8(1);
            write_string(writer, value, "name")
        }
        None => {
            writer.write_u8(0);
            Ok(())
        }
    }
}

fn write_string(writer: &mut Writer, value: &str, field: &str) -> Result<(), SessionError> {
    writer
        .write_string_u16(value)
        .map_err(|_| SessionError::InvalidSnapshot(format!("{field} is too long to serialize")))
}

fn write_optional_u64(writer: &mut Writer, value: Option<u64>) {
    match value {
        Some(value) => {
            writer.write_u8(1);
            writer.write_u64_le(value);
        }
        None => writer.write_u8(0),
    }
}

fn read_u64(reader: &mut Reader<'_>) -> Result<u64, SessionError> {
    reader.read_u64_le().map_err(|error| codec_metadata(&error))
}

fn read_string(reader: &mut Reader<'_>) -> Result<String, SessionError> {
    reader
        .read_string_u16()
        .map(str::to_owned)
        .map_err(|error| codec_metadata(&error))
}

fn read_optional_string(
    reader: &mut Reader<'_>,
    max_name_bytes: usize,
) -> Result<Option<String>, SessionError> {
    match read_flag(reader)? {
        false => Ok(None),
        true => {
            let name = read_string(reader)?;
            if name.len() > max_name_bytes {
                return Err(SessionError::CorruptMetadata(format!(
                    "metadata name is {} bytes, limit is {max_name_bytes}",
                    name.len()
                )));
            }
            if name.chars().any(char::is_control) {
                return Err(SessionError::CorruptMetadata(
                    "metadata name contains a control character".to_owned(),
                ));
            }
            Ok(Some(name))
        }
    }
}

fn read_optional_u64(reader: &mut Reader<'_>) -> Result<Option<u64>, SessionError> {
    match read_flag(reader)? {
        false => Ok(None),
        true => Ok(Some(read_u64(reader)?)),
    }
}

fn read_flag(reader: &mut Reader<'_>) -> Result<bool, SessionError> {
    match reader.read_u8().map_err(|error| codec_metadata(&error))? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(SessionError::CorruptMetadata(format!(
            "invalid optional-field flag {other}"
        ))),
    }
}

fn codec_metadata(error: &CodecError) -> SessionError {
    SessionError::CorruptMetadata(format!("metadata record is malformed: {error}"))
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{BackendId, BackendSnapshot};
    use chronofork_core::{SessionId, SnapshotId, StateHash, Tick, hash_bytes};

    use super::{
        METADATA_MAGIC, MetadataRecord, RecordKind, STORAGE_METADATA_FORMAT_VERSION,
        SessionMetadata, SnapshotMetadata,
    };
    use crate::error::SessionError;
    use crate::snapshot::Snapshot;

    fn session_metadata() -> SessionMetadata {
        SessionMetadata {
            storage_session_key: 3,
            recorded_session_id: 1,
            name: Some("baseline".to_owned()),
            parent_session_key: Some(2),
            origin_snapshot_key: Some(7),
            next_snapshot_key: 5,
            next_creation_sequence: 4,
            created_at_unix_ms: Some(1_700_000_000_000),
        }
    }

    fn snapshot_metadata() -> SnapshotMetadata {
        SnapshotMetadata {
            storage_session_key: 3,
            storage_snapshot_key: 4,
            recorded_snapshot_id: 4,
            recorded_session_id: 1,
            name: Some("named".to_owned()),
            backend_id: "chronofork.test.synthetic".to_owned(),
            backend_version: "0.1.0".to_owned(),
            backend_format_version: 1,
            tick: 42,
            config_hash: [0x11; 32],
            state_hash: [0x22; 32],
            checkpoint_length: 120,
            checkpoint_sha256: [0x33; 32],
            parent_session_key: Some(2),
            origin_snapshot_key: Some(7),
            creation_sequence: 9,
            created_at_unix_ms: Some(1_700_000_000_000),
        }
    }

    fn snapshot() -> Snapshot {
        let backend = BackendSnapshot::new(
            BackendId::new("chronofork.test.synthetic"),
            "0.1.0",
            1,
            Tick::from_raw(42),
            StateHash::from_bytes([0x11; 32]),
            StateHash::from_bytes([0x22; 32]),
            vec![1, 2, 3, 4],
        );
        Snapshot::new(SnapshotId::from_raw(4), SessionId::from_raw(1), backend)
    }

    fn rewrite_checksum(bytes: &mut [u8]) {
        let body_len = bytes.len() - 32;
        let checksum = hash_bytes(&bytes[..body_len]);
        bytes[body_len..].copy_from_slice(checksum.as_bytes());
    }

    #[test]
    fn both_record_kinds_round_trip() {
        for record in [
            MetadataRecord::Session(session_metadata()),
            MetadataRecord::Snapshot(snapshot_metadata()),
        ] {
            let bytes = record.encode().expect("metadata encodes");
            let decoded = MetadataRecord::decode(&bytes, 256).expect("metadata decodes");
            assert_eq!(decoded, record);
        }
        assert_eq!(
            MetadataRecord::Session(session_metadata()).kind(),
            RecordKind::Session
        );
        assert_eq!(
            MetadataRecord::Snapshot(snapshot_metadata()).kind(),
            RecordKind::Snapshot
        );
    }

    #[test]
    fn metadata_checksum_magic_and_trailing_bytes_are_corrupt() {
        let bytes = MetadataRecord::Session(session_metadata())
            .encode()
            .expect("metadata encodes");

        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xff;
        assert!(matches!(
            MetadataRecord::decode(&flipped, 256),
            Err(SessionError::CorruptMetadata(_))
        ));

        let mut magic = bytes.clone();
        magic[0] ^= 0xff;
        rewrite_checksum(&mut magic);
        assert!(matches!(
            MetadataRecord::decode(&magic, 256),
            Err(SessionError::CorruptMetadata(_))
        ));

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            MetadataRecord::decode(&trailing, 256),
            Err(SessionError::CorruptMetadata(_))
        ));
    }

    #[test]
    fn unsupported_metadata_version_is_typed() {
        let mut bytes = MetadataRecord::Session(session_metadata())
            .encode()
            .expect("metadata encodes");
        assert_eq!(&bytes[..8], &METADATA_MAGIC);
        bytes[8..10].copy_from_slice(&(STORAGE_METADATA_FORMAT_VERSION + 1).to_le_bytes());
        rewrite_checksum(&mut bytes);
        assert_eq!(
            MetadataRecord::decode(&bytes, 256),
            Err(SessionError::UnsupportedMetadataVersion {
                found: STORAGE_METADATA_FORMAT_VERSION + 1,
                supported: STORAGE_METADATA_FORMAT_VERSION,
            })
        );
    }

    #[test]
    fn mirrored_envelope_fields_are_rejected_by_name() {
        let snapshot = snapshot();
        let mut metadata = snapshot_metadata();
        metadata.checkpoint_length = snapshot.to_bytes().expect("serialize").len() as u64;
        metadata.checkpoint_sha256 =
            *hash_bytes(&snapshot.to_bytes().expect("serialize")).as_bytes();
        assert_eq!(metadata.validate_against(&snapshot, 3, 4), Ok(()));

        metadata.tick += 1;
        assert!(
            matches!(
                metadata.validate_against(&snapshot, 3, 4),
                Err(SessionError::MetadataMismatch(message)) if message.contains("tick")
            ),
            "a mirrored tick mismatch must be a metadata mismatch"
        );
    }

    #[test]
    fn identity_binding_is_rejected_before_mirrored_fields() {
        let snapshot = snapshot();
        let metadata = snapshot_metadata();
        assert!(matches!(
            metadata.validate_against(&snapshot, 3, 5),
            Err(SessionError::IdentityMismatch(_))
        ));
    }

    #[test]
    fn host_timestamps_never_enter_canonical_snapshot_bytes() {
        let snapshot = snapshot();
        let before = snapshot.to_bytes().expect("serialize");
        let mut metadata = snapshot_metadata();
        metadata.created_at_unix_ms = Some(0);
        let encoded = MetadataRecord::Snapshot(metadata)
            .encode()
            .expect("metadata encodes");
        let after = snapshot.to_bytes().expect("serialize");
        assert_eq!(before, after);
        assert!(
            !before
                .windows(encoded.len().min(8))
                .any(|window| window == METADATA_MAGIC)
        );
    }

    #[test]
    fn arbitrary_metadata_bytes_never_panic() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for length in 0..512 {
            let mut bytes = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push((state >> 24).to_le_bytes()[0]);
            }
            assert!(MetadataRecord::decode(&bytes, 256).is_err());
        }
    }
}
