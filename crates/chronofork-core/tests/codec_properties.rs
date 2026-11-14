#![allow(clippy::expect_used)]

//! Adversarial property tests for the shared byte codec.
//!
//! Every binary parser in the workspace reads through this codec. Untrusted
//! bytes and arbitrary field sequences must either produce the original values
//! or a typed [`chronofork_core::codec::CodecError`]; no input may panic.

use chronofork_core::codec::{CodecError, Reader, Writer};
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    #[test]
    fn arbitrary_read_sequences_never_panic(
        bytes in vec(any::<u8>(), 0..512),
        operations in vec(0_u8..10, 0..32),
    ) {
        let mut reader = Reader::new(&bytes);
        for operation in operations {
            let result = match operation {
                0 => reader.read_u8().map(u64::from),
                1 => reader.read_u16_le().map(u64::from),
                2 => reader.read_u16_be().map(u64::from),
                3 => reader.read_u32_le().map(u64::from),
                4 => reader.read_u32_be().map(u64::from),
                5 => reader.read_u64_le(),
                6 => reader.read_u64_be(),
                7 => reader.read_string_u16().map(|text| text.len() as u64),
                8 => reader.read_array::<7>().map(|array| u64::from(array[0])),
                _ => reader
                    .read_exact(usize::from(operation))
                    .map(|slice| slice.len() as u64),
            };
            if let Err(error) = result {
                // Rendering a typed error is part of the contract.
                let _ = error.to_string();
            }
            let _ = reader.remaining();
            let _ = reader.finish();
        }
    }

    #[test]
    fn read_exact_returns_the_requested_length_or_a_typed_error(
        bytes in vec(any::<u8>(), 0..256),
        length in any::<usize>(),
    ) {
        let mut reader = Reader::new(&bytes);
        match reader.read_exact(length) {
            Ok(slice) => prop_assert_eq!(slice.len(), length),
            Err(error) => {
                let typed = matches!(
                    error,
                    CodecError::Truncated { .. } | CodecError::LengthOverflow
                );
                prop_assert!(typed);
            }
        }
    }

    #[test]
    fn arbitrary_field_sequences_round_trip_in_order(
        byte in any::<u8>(),
        le_u16 in any::<u16>(),
        be_u16 in any::<u16>(),
        le_u32 in any::<u32>(),
        be_u32 in any::<u32>(),
        le_u64 in any::<u64>(),
        be_u64 in any::<u64>(),
        raw in vec(any::<u8>(), 0..96),
        text in "[A-Za-z0-9_-]{1,64}",
    ) {
        let mut writer = Writer::with_capacity(0);
        writer.write_u8(byte);
        writer.write_u16_le(le_u16);
        writer.write_u16_be(be_u16);
        writer.write_u32_le(le_u32);
        writer.write_u32_be(be_u32);
        writer.write_u64_le(le_u64);
        writer.write_u64_be(be_u64);
        writer.write_bytes(&raw);
        writer
            .write_string_u16(&text)
            .expect("generated string fits the u16 length prefix");
        let encoded = writer.into_bytes();

        let mut reader = Reader::new(&encoded);
        prop_assert_eq!(reader.read_u8().expect("u8"), byte);
        prop_assert_eq!(reader.read_u16_le().expect("le u16"), le_u16);
        prop_assert_eq!(reader.read_u16_be().expect("be u16"), be_u16);
        prop_assert_eq!(reader.read_u32_le().expect("le u32"), le_u32);
        prop_assert_eq!(reader.read_u32_be().expect("be u32"), be_u32);
        prop_assert_eq!(reader.read_u64_le().expect("le u64"), le_u64);
        prop_assert_eq!(reader.read_u64_be().expect("be u64"), be_u64);
        prop_assert_eq!(
            reader.read_exact(raw.len()).expect("raw bytes"),
            raw.as_slice()
        );
        prop_assert_eq!(
            reader.read_string_u16().expect("length-prefixed string"),
            text.as_str()
        );
        prop_assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn overlong_strings_fail_without_overflowing_the_prefix(extra in 1_usize..1024) {
        let text = "a".repeat(usize::from(u16::MAX) + extra);
        let mut writer = Writer::with_capacity(0);
        prop_assert_eq!(
            writer.write_string_u16(&text),
            Err(CodecError::LengthOverflow)
        );
        prop_assert!(writer.as_slice().is_empty());
    }
}
