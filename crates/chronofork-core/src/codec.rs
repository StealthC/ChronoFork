//! Bounded, explicit byte reader and writer for ChronoFork binary formats.
//!
//! Every hand-written parser and serializer in the workspace uses this module so
//! bounds checks, endianness and length-prefixed string handling have exactly one
//! implementation. Readers operate on untrusted input and never panic: every
//! access is bounds-checked and reports a typed [`CodecError`].
//!
//! Fixed-width integers are read and written with the endianness the format
//! declares. Length-prefixed strings use a little-endian `u16` byte length and
//! must be non-empty valid UTF-8.

use std::error::Error;
use std::fmt;
use std::str::Utf8Error;

/// A malformed byte sequence or a value that cannot be serialized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    /// A read needed more bytes than the input has left.
    Truncated {
        /// Number of bytes requested.
        needed: usize,
        /// Offset at which the read started.
        offset: usize,
        /// Bytes available at `offset`.
        remaining: usize,
    },
    /// A length or offset computation overflowed `usize`, or a value does not
    /// fit the width of its serialized field.
    LengthOverflow,
    /// Bytes remain after the last expected field.
    TrailingBytes {
        /// Number of unexpected bytes.
        count: usize,
    },
    /// A length-prefixed string is not valid UTF-8.
    InvalidUtf8(Utf8Error),
    /// A length-prefixed string has zero length.
    EmptyString,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                needed,
                offset,
                remaining,
            } => write!(
                f,
                "byte sequence truncated: needed {needed} bytes at offset {offset}, \
                 only {remaining} remain"
            ),
            Self::LengthOverflow => f.write_str("byte sequence length overflow"),
            Self::TrailingBytes { count } => {
                write!(f, "byte sequence has {count} trailing byte(s)")
            }
            Self::InvalidUtf8(error) => write!(f, "byte sequence string is not UTF-8: {error}"),
            Self::EmptyString => f.write_str("byte sequence string must not be empty"),
        }
    }
}

impl Error for CodecError {}

/// A bounds-checked cursor over a borrowed byte slice.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// Creates a reader positioned at the start of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Returns how many bytes have not been read yet.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// Reads exactly `length` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when the input is too short and
    /// [`CodecError::LengthOverflow`] when the offset arithmetic overflows.
    pub fn read_exact(&mut self, length: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(CodecError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(CodecError::Truncated {
                needed: length,
                offset: self.position,
                remaining: self.remaining(),
            });
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    /// Reads exactly `N` bytes as an array.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than `N` bytes remain.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let slice = self.read_exact(N)?;
        let mut output = [0_u8; N];
        output.copy_from_slice(slice);
        Ok(output)
    }

    /// Reads one byte.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when no byte remains.
    pub fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.read_array::<1>()?[0])
    }

    /// Reads a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than two bytes remain.
    pub fn read_u16_le(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    /// Reads a big-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than two bytes remain.
    pub fn read_u16_be(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(self.read_array::<2>()?))
    }

    /// Reads a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than four bytes remain.
    pub fn read_u32_le(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    /// Reads a big-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than four bytes remain.
    pub fn read_u32_be(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.read_array::<4>()?))
    }

    /// Reads a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than eight bytes remain.
    pub fn read_u64_le(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    /// Reads a big-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when fewer than eight bytes remain.
    pub fn read_u64_be(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_be_bytes(self.read_array::<8>()?))
    }

    /// Reads a little-endian-`u16`-length-prefixed, non-empty UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Truncated`] when the prefix or body is short,
    /// [`CodecError::InvalidUtf8`] for malformed UTF-8 and
    /// [`CodecError::EmptyString`] for a zero-length body.
    pub fn read_string_u16(&mut self) -> Result<&'a str, CodecError> {
        let length = usize::from(self.read_u16_le()?);
        let bytes = self.read_exact(length)?;
        let text = std::str::from_utf8(bytes).map_err(CodecError::InvalidUtf8)?;
        if text.is_empty() {
            return Err(CodecError::EmptyString);
        }
        Ok(text)
    }

    /// Requires that the whole input has been consumed.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::TrailingBytes`] when bytes remain.
    pub fn finish(&self) -> Result<(), CodecError> {
        let count = self.remaining();
        if count == 0 {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes { count })
        }
    }
}

/// A growable byte buffer with fixed-width, explicitly endian writes.
#[derive(Debug)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    /// Creates a writer with room for at least `capacity` bytes.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Appends raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Appends one byte.
    pub fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Appends a little-endian `u16`.
    pub fn write_u16_le(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Appends a big-endian `u16`.
    pub fn write_u16_be(&mut self, value: u16) {
        self.write_bytes(&value.to_be_bytes());
    }

    /// Appends a little-endian `u32`.
    pub fn write_u32_le(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Appends a big-endian `u32`.
    pub fn write_u32_be(&mut self, value: u32) {
        self.write_bytes(&value.to_be_bytes());
    }

    /// Appends a little-endian `u64`.
    pub fn write_u64_le(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Appends a big-endian `u64`.
    pub fn write_u64_be(&mut self, value: u64) {
        self.write_bytes(&value.to_be_bytes());
    }

    /// Appends a string with a little-endian `u16` byte-length prefix.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::LengthOverflow`] when the UTF-8 length does not fit
    /// in a `u16`.
    pub fn write_string_u16(&mut self, value: &str) -> Result<(), CodecError> {
        let length = u16::try_from(value.len()).map_err(|_| CodecError::LengthOverflow)?;
        self.write_u16_le(length);
        self.write_bytes(value.as_bytes());
        Ok(())
    }

    /// Consumes the writer and returns the accumulated bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Borrows the accumulated bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{CodecError, Reader, Writer};

    #[test]
    fn round_trip_preserves_field_order_and_endianness() {
        let mut writer = Writer::with_capacity(64);
        writer.write_bytes(b"CF");
        writer.write_u8(0x7f);
        writer.write_u16_le(0x1234);
        writer.write_u16_be(0x1234);
        writer.write_u32_le(0x89ab_cdef);
        writer.write_u32_be(0x89ab_cdef);
        writer.write_u64_le(0x0123_4567_89ab_cdef);
        writer.write_u64_be(0x0123_4567_89ab_cdef);
        writer
            .write_string_u16("chronofork")
            .expect("string fits u16");
        let bytes = writer.into_bytes();

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.read_array::<2>().expect("prefix"), *b"CF");
        assert_eq!(reader.read_u8().expect("u8"), 0x7f);
        assert_eq!(reader.read_u16_le().expect("le u16"), 0x1234);
        assert_eq!(reader.read_u16_be().expect("be u16"), 0x1234);
        assert_eq!(reader.read_u32_le().expect("le u32"), 0x89ab_cdef);
        assert_eq!(reader.read_u32_be().expect("be u32"), 0x89ab_cdef);
        assert_eq!(reader.read_u64_le().expect("le u64"), 0x0123_4567_89ab_cdef);
        assert_eq!(reader.read_u64_be().expect("be u64"), 0x0123_4567_89ab_cdef);
        assert_eq!(reader.read_string_u16().expect("string"), "chronofork");
        assert_eq!(reader.remaining(), 0);
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn truncation_reports_offset_and_remaining_bytes() {
        let mut reader = Reader::new(&[0x01, 0x02, 0x03]);
        assert_eq!(reader.read_u16_le().expect("first word"), 0x0201);
        assert_eq!(
            reader.read_u16_le(),
            Err(CodecError::Truncated {
                needed: 2,
                offset: 2,
                remaining: 1,
            })
        );
        assert_eq!(reader.remaining(), 1);
        assert_eq!(reader.finish(), Err(CodecError::TrailingBytes { count: 1 }));
    }

    #[test]
    fn length_overflow_is_a_typed_error() {
        let mut reader = Reader::new(&[0x01, 0x02]);
        assert_eq!(reader.read_u8().expect("first byte"), 0x01);
        assert_eq!(
            reader.read_exact(usize::MAX),
            Err(CodecError::LengthOverflow)
        );
    }

    #[test]
    fn invalid_utf8_and_empty_strings_are_rejected() {
        let mut invalid = Reader::new(&[0x02, 0x00, 0xff, 0xfe]);
        assert!(matches!(
            invalid.read_string_u16(),
            Err(CodecError::InvalidUtf8(_))
        ));

        let mut empty = Reader::new(&[0x00, 0x00]);
        assert_eq!(empty.read_string_u16(), Err(CodecError::EmptyString));
    }

    #[test]
    fn writer_rejects_strings_that_do_not_fit_u16_length() {
        let too_long = "a".repeat(usize::from(u16::MAX) + 1);
        let mut writer = Writer::with_capacity(0);
        assert_eq!(
            writer.write_string_u16(&too_long),
            Err(CodecError::LengthOverflow)
        );
    }

    #[test]
    fn error_display_is_explicit() {
        assert_eq!(
            CodecError::LengthOverflow.to_string(),
            "byte sequence length overflow"
        );
        assert_eq!(
            CodecError::TrailingBytes { count: 2 }.to_string(),
            "byte sequence has 2 trailing byte(s)"
        );
        assert_eq!(
            CodecError::EmptyString.to_string(),
            "byte sequence string must not be empty"
        );
    }
}
