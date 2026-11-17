//! A strict, bounded, canonical JSON codec owned by the service layer.
//!
//! The codec exists so every transport compares bytes rather than field subsets:
//! object keys are sorted on output, integers are the only number type, duplicate
//! keys are rejected and no insignificant whitespace is produced. It is deliberately
//! small and dependency-free; see ADR-0012 decision 4.
//!
//! The accepted profile is exactly architecture §2: `null`, booleans, integers in
//! `[-(2^53 - 1), 2^53 - 1]`, UTF-8 strings (with `\uXXXX` escapes, including
//! surrogate pairs), arrays and objects with unique keys. Floating point numbers,
//! exponent notation, raw control characters, invalid UTF-8, unpaired surrogates,
//! duplicate keys, excessive nesting and oversized documents are typed rejections,
//! never panics or truncation.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::{ErrorCode, ServiceError};
use crate::limits::ServiceLimits;

/// Largest integer that every JSON consumer represents exactly (`2^53 - 1`).
pub const MAX_INTEGER: i64 = 9_007_199_254_740_991;

/// [`MAX_INTEGER`] as an unsigned value.
pub const MAX_INTEGER_U64: u64 = 9_007_199_254_740_991;

/// One JSON value in the service's canonical profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON `true` or `false`.
    Bool(bool),
    /// A signed integer in the portable range.
    Int(i64),
    /// A UTF-8 string.
    String(String),
    /// An ordered JSON array.
    Array(Vec<JsonValue>),
    /// A JSON object with unique keys.
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    /// Builds an object from key/value pairs.
    ///
    /// Keys are sorted by the backing map, so insertion order never reaches the wire.
    pub fn object<I, K>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, JsonValue)>,
        K: Into<String>,
    {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    /// Builds an array from its values.
    pub fn array(values: impl IntoIterator<Item = JsonValue>) -> Self {
        Self::Array(values.into_iter().collect())
    }

    /// Builds a JSON string.
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// Builds a JSON integer.
    #[must_use]
    pub const fn int(value: i64) -> Self {
        Self::Int(value)
    }

    /// Builds a JSON boolean.
    #[must_use]
    pub const fn from_bool(value: bool) -> Self {
        Self::Bool(value)
    }

    /// Returns the value for `key` when this is an object.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Returns the string when this is a JSON string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// Returns the integer when this is a JSON integer.
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// Returns the integer when this is a non-negative JSON integer.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Int(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// Returns the boolean when this is a JSON boolean.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Returns the array when this is a JSON array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// Returns the map when this is a JSON object.
    #[must_use]
    pub fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }
}

/// Parses one JSON document under the canonical profile.
///
/// # Errors
///
/// Returns a typed [`ServiceError`] for oversized input, invalid UTF-8, malformed
/// syntax, floating point numbers, integers outside the portable range, duplicate
/// object keys, invalid escapes, unpaired surrogates and excessive nesting. The
/// input bytes never appear in the message.
pub fn parse(bytes: &[u8], limits: &ServiceLimits) -> Result<JsonValue, ServiceError> {
    if bytes.len() > limits.max_request_bytes {
        return Err(limit("input exceeds the configured byte limit"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("input is not valid UTF-8"))?;
    Parser {
        text,
        bytes,
        position: 0,
        max_depth: limits.json_max_depth,
    }
    .parse_document()
}

/// Serializes one value to canonical bytes.
///
/// Object keys are sorted bytewise, no insignificant whitespace is emitted and the
/// configured response and depth limits are enforced while writing, so an oversized
/// document fails before more than one bounded chunk is appended.
///
/// # Errors
///
/// Returns [`ErrorCode::LimitExceeded`] when the canonical document exceeds
/// `limits.max_response_bytes` or nests deeper than `limits.json_max_depth`, and
/// [`ErrorCode::InvalidRequest`] for an integer outside the portable range.
pub fn to_canonical_bytes(
    value: &JsonValue,
    limits: &ServiceLimits,
) -> Result<Vec<u8>, ServiceError> {
    let mut output = Vec::new();
    write_value(value, &mut output, 0, limits)?;
    Ok(output)
}

fn invalid(message: &'static str) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

fn limit(message: &'static str) -> ServiceError {
    ServiceError::new(ErrorCode::LimitExceeded, message)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    max_depth: usize,
}

impl Parser<'_> {
    fn parse_document(mut self) -> Result<JsonValue, ServiceError> {
        self.skip_whitespace();
        let value = self.parse_value(0)?;
        self.skip_whitespace();
        if self.position != self.bytes.len() {
            return Err(invalid("unexpected trailing content"));
        }
        Ok(value)
    }

    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, ServiceError> {
        match self.byte() {
            None => Err(invalid("unexpected end of input")),
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(JsonValue::String(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", JsonValue::Bool(true)),
            Some(b'f') => self.parse_literal("false", JsonValue::Bool(false)),
            Some(b'n') => self.parse_literal("null", JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(invalid("unexpected character")),
        }
    }

    fn parse_literal(
        &mut self,
        literal: &'static str,
        value: JsonValue,
    ) -> Result<JsonValue, ServiceError> {
        let end = self.position + literal.len();
        if self.bytes.get(self.position..end) == Some(literal.as_bytes()) {
            self.position = end;
            Ok(value)
        } else {
            Err(invalid("unexpected character"))
        }
    }

    fn enter(&self, depth: usize) -> Result<(), ServiceError> {
        if depth >= self.max_depth {
            return Err(limit("JSON nesting exceeds the configured depth limit"));
        }
        Ok(())
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue, ServiceError> {
        self.enter(depth)?;
        self.position += 1;
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.byte() == Some(b'}') {
            self.position += 1;
            return Ok(JsonValue::Object(map));
        }
        loop {
            self.skip_whitespace();
            if self.byte() != Some(b'"') {
                return Err(invalid("expected an object key"));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.byte() != Some(b':') {
                return Err(invalid("expected `:` after an object key"));
            }
            self.position += 1;
            self.skip_whitespace();
            let value = self.parse_value(depth + 1)?;
            if map.insert(key, value).is_some() {
                return Err(invalid("duplicate object key"));
            }
            self.skip_whitespace();
            match self.byte() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(JsonValue::Object(map));
                }
                _ => return Err(invalid("expected `,` or `}` in an object")),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue, ServiceError> {
        self.enter(depth)?;
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.byte() == Some(b']') {
            self.position += 1;
            return Ok(JsonValue::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            match self.byte() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(JsonValue::Array(values));
                }
                _ => return Err(invalid("expected `,` or `]` in an array")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ServiceError> {
        // The caller has verified that the current byte is `"`.
        self.position += 1;
        let mut output = String::new();
        loop {
            match self.byte() {
                None => return Err(invalid("unexpected end of input")),
                Some(b'"') => {
                    self.position += 1;
                    return Ok(output);
                }
                Some(b'\\') => {
                    self.position += 1;
                    self.parse_escape(&mut output)?;
                }
                Some(byte) if byte < 0x20 => {
                    return Err(invalid("control character in a string"));
                }
                Some(byte) if byte < 0x80 => {
                    output.push(char::from(byte));
                    self.position += 1;
                }
                Some(_) => {
                    // The whole document is valid UTF-8, so the next character is
                    // always decodable from the remaining text.
                    let character = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or_else(|| invalid("unexpected end of input"))?;
                    output.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn parse_escape(&mut self, output: &mut String) -> Result<(), ServiceError> {
        let byte = self
            .byte()
            .ok_or_else(|| invalid("unexpected end of input"))?;
        self.position += 1;
        match byte {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000C}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => output.push(self.parse_unicode_escape()?),
            _ => return Err(invalid("invalid escape sequence")),
        }
        Ok(())
    }

    fn parse_unicode_escape(&mut self) -> Result<char, ServiceError> {
        let unit = self.parse_hex4()?;
        match unit {
            0xD800..=0xDBFF => {
                if self.byte() != Some(b'\\') {
                    return Err(invalid("unpaired surrogate in a string"));
                }
                self.position += 1;
                if self.byte() != Some(b'u') {
                    return Err(invalid("unpaired surrogate in a string"));
                }
                self.position += 1;
                let low = self.parse_hex4()?;
                if !(0xDC00..=0xDFFF).contains(&low) {
                    return Err(invalid("unpaired surrogate in a string"));
                }
                let code = 0x1_0000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                char::from_u32(code).ok_or_else(|| invalid("invalid unicode escape"))
            }
            0xDC00..=0xDFFF => Err(invalid("unpaired surrogate in a string")),
            _ => char::from_u32(unit).ok_or_else(|| invalid("invalid unicode escape")),
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, ServiceError> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let byte = self
                .byte()
                .ok_or_else(|| invalid("unexpected end of input"))?;
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(invalid("invalid unicode escape")),
            };
            value = (value << 4) | digit;
            self.position += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<JsonValue, ServiceError> {
        let start = self.position;
        if self.byte() == Some(b'-') {
            self.position += 1;
        }
        match self.byte() {
            Some(b'0') => {
                self.position += 1;
                if matches!(self.byte(), Some(b'0'..=b'9')) {
                    return Err(invalid("numbers must not have leading zeroes"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.byte(), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(invalid("expected a digit")),
        }
        if matches!(self.byte(), Some(b'.' | b'e' | b'E')) {
            return Err(invalid("floating point numbers are not supported"));
        }
        let text = self
            .text
            .get(start..self.position)
            .ok_or_else(|| invalid("unexpected character"))?;
        let value = text
            .parse::<i64>()
            .map_err(|_| invalid("integer outside the canonical range"))?;
        if !(-MAX_INTEGER..=MAX_INTEGER).contains(&value) {
            return Err(invalid("integer outside the canonical range"));
        }
        Ok(JsonValue::Int(value))
    }
}

fn write_value(
    value: &JsonValue,
    output: &mut Vec<u8>,
    depth: usize,
    limits: &ServiceLimits,
) -> Result<(), ServiceError> {
    match value {
        JsonValue::Null => push(output, b"null", limits),
        JsonValue::Bool(true) => push(output, b"true", limits),
        JsonValue::Bool(false) => push(output, b"false", limits),
        JsonValue::Int(value) => {
            if !(-MAX_INTEGER..=MAX_INTEGER).contains(value) {
                return Err(invalid("integer outside the canonical range"));
            }
            push(output, value.to_string().as_bytes(), limits)
        }
        JsonValue::String(value) => write_string(value, output, limits),
        JsonValue::Array(values) => {
            if depth >= limits.json_max_depth {
                return Err(limit("JSON nesting exceeds the configured depth limit"));
            }
            push(output, b"[", limits)?;
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    push(output, b",", limits)?;
                }
                write_value(item, output, depth + 1, limits)?;
            }
            push(output, b"]", limits)
        }
        JsonValue::Object(map) => {
            if depth >= limits.json_max_depth {
                return Err(limit("JSON nesting exceeds the configured depth limit"));
            }
            push(output, b"{", limits)?;
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    push(output, b",", limits)?;
                }
                write_string(key, output, limits)?;
                push(output, b":", limits)?;
                write_value(item, output, depth + 1, limits)?;
            }
            push(output, b"}", limits)
        }
    }
}

fn push(output: &mut Vec<u8>, bytes: &[u8], limits: &ServiceLimits) -> Result<(), ServiceError> {
    if output.len().saturating_add(bytes.len()) > limits.max_response_bytes {
        return Err(limit("output exceeds the configured byte limit"));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn write_string(
    value: &str,
    output: &mut Vec<u8>,
    limits: &ServiceLimits,
) -> Result<(), ServiceError> {
    push(output, b"\"", limits)?;
    let bytes = value.as_bytes();
    let mut segment_start = 0;
    for (index, character) in value.char_indices() {
        let mut escape = String::new();
        match character {
            '"' => escape.push_str("\\\""),
            '\\' => escape.push_str("\\\\"),
            '\u{0008}' => escape.push_str("\\b"),
            '\u{000C}' => escape.push_str("\\f"),
            '\n' => escape.push_str("\\n"),
            '\r' => escape.push_str("\\r"),
            '\t' => escape.push_str("\\t"),
            character if character < ' ' => {
                // Writing to a String cannot fail.
                let _ = write!(escape, "\\u{:04x}", u32::from(character));
            }
            _ => continue,
        }
        push(output, &bytes[segment_start..index], limits)?;
        push(output, escape.as_bytes(), limits)?;
        segment_start = index + character.len_utf8();
    }
    push(output, &bytes[segment_start..], limits)?;
    push(output, b"\"", limits)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::{JsonValue, MAX_INTEGER, parse, to_canonical_bytes};
    use crate::error::ErrorCode;
    use crate::limits::ServiceLimits;

    fn canonical(text: &str) -> Result<String, crate::error::ServiceError> {
        let limits = ServiceLimits::default();
        let value = parse(text.as_bytes(), &limits)?;
        let bytes = to_canonical_bytes(&value, &limits)?;
        String::from_utf8(bytes).map_err(|_| crate::error::ServiceError::internal("utf8"))
    }

    fn rejection(text: &str, code: ErrorCode) {
        let limits = ServiceLimits::default();
        let error = parse(text.as_bytes(), &limits)
            .expect_err("document must be rejected: a rejected document is the contract");
        assert_eq!(error.code(), code, "{text:?}: {error}");
    }

    #[test]
    fn canonical_output_sorts_keys_and_strips_whitespace() {
        assert_eq!(
            canonical(" { \"b\" : 2 , \"a\" : [ true , null , \"x\" ] } ").expect("canonical"),
            "{\"a\":[true,null,\"x\"],\"b\":2}"
        );
    }

    #[test]
    fn parse_serialize_parse_is_stable() {
        let limits = ServiceLimits::default();
        let first = "{\"z\":[1,-2,3],\"a\":{\"k\":\"v\"}}";
        let value = parse(first.as_bytes(), &limits).expect("parse");
        let bytes = to_canonical_bytes(&value, &limits).expect("serialize");
        let reparsed = parse(&bytes, &limits).expect("reparse");
        assert_eq!(reparsed, value);
        assert_eq!(
            to_canonical_bytes(&reparsed, &limits).expect("reserialize"),
            bytes
        );
    }

    #[test]
    fn integer_bounds_are_enforced() {
        let limits = ServiceLimits::default();
        let maximum = format!("{MAX_INTEGER}");
        assert_eq!(
            parse(maximum.as_bytes(), &limits).expect("max").as_i64(),
            Some(MAX_INTEGER)
        );
        let minimum = format!("-{MAX_INTEGER}");
        assert_eq!(
            parse(minimum.as_bytes(), &limits).expect("min").as_i64(),
            Some(-MAX_INTEGER)
        );
        rejection("9007199254740992", ErrorCode::InvalidRequest);
        rejection("-9007199254740992", ErrorCode::InvalidRequest);
        rejection("9223372036854775808", ErrorCode::InvalidRequest);
    }

    #[test]
    fn floating_point_and_exponent_forms_are_rejected() {
        for text in ["1.5", "-0.0", "1e3", "1E3", "1e+3", "01", "+1", ".5", "1."] {
            rejection(text, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        rejection("{\"a\":1,\"a\":2}", ErrorCode::InvalidRequest);
        rejection("{\"a\":{\"b\":1,\"b\":1}}", ErrorCode::InvalidRequest);
    }

    #[test]
    fn malformed_syntax_is_rejected() {
        for text in [
            "",
            " ",
            "{",
            "[",
            "\"unterminated",
            "{\"a\"}",
            "{\"a\":}",
            "[1,]",
            "{\"a\":1,}",
            "tru",
            "nul",
            "1 2",
            "{\"a\":1}x",
        ] {
            rejection(text, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn control_characters_and_invalid_utf8_are_rejected() {
        rejection("\"a\u{1}b\"", ErrorCode::InvalidRequest);
        rejection("\"a\tb\"", ErrorCode::InvalidRequest);
        rejection("\"a\nb\"", ErrorCode::InvalidRequest);
        let limits = ServiceLimits::default();
        let bytes = [b'"', 0x80, b'"'];
        assert_eq!(
            parse(&bytes, &limits).expect_err("invalid utf8").code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            parse(&[0xFF, 0xFE], &limits)
                .expect_err("invalid utf8")
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn unicode_escapes_and_surrogates_round_trip() {
        assert_eq!(canonical("\"\\u00e9\"").expect("escaped"), "\"é\"");
        assert_eq!(
            canonical("\"\\uD83D\\uDE00\"").expect("surrogate pair"),
            "\"😀\""
        );
        assert_eq!(canonical("\"\\/\"").expect("slash"), "\"/\"");
        assert_eq!(canonical("\"a\\\"b\"").expect("quote"), "\"a\\\"b\"");
        assert_eq!(canonical("\"a\\\\b\"").expect("backslash"), "\"a\\\\b\"");
        rejection("\"\\uD800\"", ErrorCode::InvalidRequest);
        rejection("\"\\uDC00\"", ErrorCode::InvalidRequest);
        rejection("\"\\uD800\\u0041\"", ErrorCode::InvalidRequest);
        rejection("\"\\uD800x\"", ErrorCode::InvalidRequest);
        rejection("\"\\u00\"", ErrorCode::InvalidRequest);
        rejection("\"\\uZZZZ\"", ErrorCode::InvalidRequest);
        rejection("\"\\q\"", ErrorCode::InvalidRequest);
    }

    #[test]
    fn depth_is_bounded_on_input_and_output() {
        let limits = ServiceLimits::default();
        let allowed = format!(
            "{}0{}",
            "[".repeat(limits.json_max_depth),
            "]".repeat(limits.json_max_depth)
        );
        parse(allowed.as_bytes(), &limits).expect("depth at the limit is accepted");
        let rejected = format!(
            "{}0{}",
            "[".repeat(limits.json_max_depth + 1),
            "]".repeat(limits.json_max_depth + 1)
        );
        assert_eq!(
            parse(rejected.as_bytes(), &limits)
                .expect_err("depth over the limit")
                .code(),
            ErrorCode::LimitExceeded
        );

        let mut deep = JsonValue::Int(0);
        for _ in 0..=limits.json_max_depth {
            deep = JsonValue::Array(vec![deep]);
        }
        assert_eq!(
            to_canonical_bytes(&deep, &limits)
                .expect_err("output depth over the limit")
                .code(),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn size_limits_are_enforced_before_and_during_writing() {
        let limits = ServiceLimits {
            max_request_bytes: 8,
            ..ServiceLimits::default()
        };
        assert_eq!(
            parse(b"{\"a\":123456789}", &limits)
                .expect_err("oversized input")
                .code(),
            ErrorCode::LimitExceeded
        );

        let limits = ServiceLimits {
            max_response_bytes: 8,
            ..ServiceLimits::default()
        };
        let value = JsonValue::string("0123456789");
        assert_eq!(
            to_canonical_bytes(&value, &limits)
                .expect_err("oversized output")
                .code(),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn accessors_report_type_mismatches() {
        let limits = ServiceLimits::default();
        let value = parse(
            b"{\"i\":-3,\"u\":3,\"s\":\"x\",\"b\":true,\"a\":[1],\"o\":{\"k\":1}}",
            &limits,
        )
        .expect("parse");
        assert_eq!(value.get("i").and_then(JsonValue::as_i64), Some(-3));
        assert_eq!(value.get("i").and_then(JsonValue::as_u64), None);
        assert_eq!(value.get("u").and_then(JsonValue::as_u64), Some(3));
        assert_eq!(value.get("s").and_then(JsonValue::as_str), Some("x"));
        assert_eq!(value.get("s").and_then(JsonValue::as_i64), None);
        assert_eq!(value.get("b").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(
            value.get("a").and_then(JsonValue::as_array).map(<[_]>::len),
            Some(1)
        );
        assert_eq!(
            value
                .get("o")
                .and_then(JsonValue::as_object)
                .and_then(|map| map.get("k"))
                .and_then(JsonValue::as_i64),
            Some(1)
        );
        assert!(JsonValue::Null.as_object().is_none());
        assert!(JsonValue::Null.get("k").is_none());
    }

    fn value_strategy() -> impl Strategy<Value = JsonValue> {
        let leaf = prop_oneof![
            Just(JsonValue::Null),
            any::<bool>().prop_map(JsonValue::Bool),
            (-MAX_INTEGER..=MAX_INTEGER).prop_map(JsonValue::Int),
            ".{0,12}".prop_map(JsonValue::String),
        ];
        leaf.prop_recursive(4, 32, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(JsonValue::Array),
                prop::collection::btree_map("[a-zA-Z0-9_.-]{0,8}", inner, 0..4)
                    .prop_map(JsonValue::Object),
            ]
        })
    }

    proptest! {
        #[test]
        fn canonical_round_trip_is_stable(value in value_strategy()) {
            let limits = ServiceLimits::default();
            let bytes = to_canonical_bytes(&value, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let parsed = parse(&bytes, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(&parsed, &value);
            let again = to_canonical_bytes(&parsed, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(again, bytes);
        }

        #[test]
        fn canonical_parsing_is_whitespace_insensitive(value in value_strategy()) {
            let limits = ServiceLimits::default();
            let bytes = to_canonical_bytes(&value, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let mut padded = Vec::new();
            padded.extend_from_slice(b" \n\t");
            padded.extend_from_slice(&bytes);
            padded.extend_from_slice(b"\r\n ");
            prop_assert_eq!(parse(&padded, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?, value);
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
            let limits = ServiceLimits::default();
            let parsed = parse(&bytes, &limits);
            if let Ok(value) = parsed {
                let canonical = to_canonical_bytes(&value, &limits)
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(parse(&canonical, &limits)
                    .map_err(|error| TestCaseError::fail(error.to_string()))?, value);
            }
        }

        #[test]
        fn canonical_output_never_contains_insignificant_whitespace(value in value_strategy()) {
            let limits = ServiceLimits::default();
            let bytes = to_canonical_bytes(&value, &limits)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let mut inside_string = false;
            let mut escaped = false;
            for byte in bytes {
                if inside_string {
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        inside_string = false;
                    }
                } else if byte == b'"' {
                    inside_string = true;
                } else {
                    prop_assert!(
                        !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'),
                        "insignificant whitespace emitted"
                    );
                }
            }
            prop_assert!(!inside_string);
        }
    }

    #[test]
    fn object_constructor_sorts_keys() {
        let value = JsonValue::object([("b", JsonValue::int(2)), ("a", JsonValue::int(1))]);
        let limits = ServiceLimits::default();
        assert_eq!(
            to_canonical_bytes(&value, &limits).expect("bytes"),
            b"{\"a\":1,\"b\":2}"
        );
        assert!(matches!(value, JsonValue::Object(map) if map.len() == 2));
        let map: BTreeMap<String, JsonValue> = BTreeMap::new();
        assert_eq!(JsonValue::Object(map.clone()), JsonValue::object(map));
    }
}
