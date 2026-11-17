//! Bearer-token authentication, principals and coarse scopes.
//!
//! Tokens are read from a `chronofork.tokens.v1` file. Authentication happens once
//! per request at the transport boundary; every operation then receives an
//! immutable [`Principal`].
//!
//! Validation is deliberately strict and complete before the service starts: one
//! principal is required, names and tokens are unique and well formed, scopes are a
//! non-empty subset of `read`, `write`, `admin`, and unknown fields are rejected
//! instead of ignored. See architecture §6.

use std::collections::BTreeSet;
use std::fmt;

use crate::error::{ErrorCode, ServiceError};
use crate::json::JsonValue;
use crate::limits::ServiceLimits;

/// Maximum number of UTF-8 bytes in a principal name (architecture §6).
const MAX_PRINCIPAL_NAME_BYTES: usize = 64;

/// Minimum number of bytes in an opaque token (architecture §6).
const MIN_TOKEN_BYTES: usize = 16;

/// Maximum number of bytes in an opaque token (architecture §6).
const MAX_TOKEN_BYTES: usize = 256;

const TOKEN_FILE_SCHEMA: &str = "chronofork.tokens.v1";

/// A coarse permission granted to a principal.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Scope {
    /// May execute read operations.
    Read,
    /// May execute read and mutating operations.
    Write,
    /// May execute every operation and observe every session, regardless of owner.
    Admin,
}

impl Scope {
    /// Returns the stable wire string for this scope.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// An authenticated caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    name: String,
    scopes: BTreeSet<Scope>,
}

impl Principal {
    /// Creates a principal with its granted scopes.
    #[must_use]
    pub fn new(name: impl Into<String>, scopes: impl IntoIterator<Item = Scope>) -> Self {
        Self {
            name: name.into(),
            scopes: scopes.into_iter().collect(),
        }
    }

    /// Returns the principal name from the token file.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether `scope` is granted, with `admin` implying every scope.
    #[must_use]
    pub fn has_scope(&self, scope: Scope) -> bool {
        self.scopes.contains(&Scope::Admin) || self.scopes.contains(&scope)
    }

    /// Returns whether this principal is an administrator.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.scopes.contains(&Scope::Admin)
    }
}

/// A validated token table.
///
/// `Debug` prints the principal names and the entry count and never renders token
/// material, so a table can be logged or embedded in a larger debug record safely.
#[derive(Clone, Default)]
pub struct TokenTable {
    entries: Vec<(String, Principal)>,
}

impl fmt::Debug for TokenTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let principals: Vec<&str> = self
            .entries
            .iter()
            .map(|(_, principal)| principal.name())
            .collect();
        formatter
            .debug_struct("TokenTable")
            .field("principals", &principals)
            .finish()
    }
}

impl TokenTable {
    /// Returns the principal for an opaque bearer token, if any.
    #[must_use]
    pub fn authenticate(&self, token: &str) -> Option<Principal> {
        self.entries
            .iter()
            .find(|(stored, _)| stored == token)
            .map(|(_, principal)| principal.clone())
    }

    /// Returns the principal with `name`, if any.
    #[must_use]
    pub fn principal(&self, name: &str) -> Option<Principal> {
        self.entries
            .iter()
            .find(|(_, principal)| principal.name() == name)
            .map(|(_, principal)| principal.clone())
    }

    /// Returns the number of configured principals.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns every validated `(principal name, opaque token)` pair in file order.
    ///
    /// A client selects a credential from the already-validated table instead of
    /// re-reading or re-parsing the token file. Tokens are returned exactly as stored;
    /// no service response, error or tool schema ever carries one.
    pub fn credentials(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(token, principal)| (principal.name(), token.as_str()))
    }
}

/// Parses and validates a `chronofork.tokens.v1` document.
///
/// The whole table is validated before it is returned; a single invalid principal
/// rejects the file so the service never starts with a partially trusted table.
///
/// The document is bounded by [`ServiceLimits::default`]'s `max_request_bytes` (1 MiB).
/// A token file is parsed before a service instance exists, so there is no per-instance
/// token-file limit; an embedder that needs a different bound caps its own read first
/// (the CLI reads the file with exactly the request limit it passes to the transport).
///
/// # Errors
///
/// Returns a typed [`ServiceError`] with [`ErrorCode::InvalidRequest`] for schema,
/// name, token, scope, duplicate or unknown-field violations and
/// [`ErrorCode::LimitExceeded`] for an oversized document. Token material is never
/// included in the message.
pub fn parse_token_file(bytes: &[u8]) -> Result<TokenTable, ServiceError> {
    let document = crate::json::parse(bytes, &ServiceLimits::default())
        .map_err(|error| invalid_owned(format!("invalid token file: {}", error.message())))?;
    let object = document
        .as_object()
        .ok_or_else(|| invalid("token file must be a JSON object"))?;
    reject_unknown(object.keys(), &["schema", "principals"])?;
    match object.get("schema").and_then(JsonValue::as_str) {
        Some(TOKEN_FILE_SCHEMA) => {}
        Some(_) => return Err(invalid("token file schema must be `chronofork.tokens.v1`")),
        None => return Err(invalid("token file is missing a string `schema`")),
    }
    let principals = object
        .get("principals")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| invalid("token file `principals` must be an array"))?;
    if principals.is_empty() {
        return Err(invalid("token file must contain at least one principal"));
    }

    let mut entries = Vec::with_capacity(principals.len());
    let mut names = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    for (index, principal) in principals.iter().enumerate() {
        let (token, parsed) = parse_principal(principal, index)?;
        if !names.insert(parsed.name().to_owned()) {
            return Err(invalid("token file contains a duplicate principal name"));
        }
        if !tokens.insert(token.clone()) {
            return Err(invalid("token file contains a duplicate token"));
        }
        entries.push((token, parsed));
    }
    Ok(TokenTable { entries })
}

/// Returns `(token, principal)` for one validated entry.
fn parse_principal(value: &JsonValue, index: usize) -> Result<(String, Principal), ServiceError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_at(index, "principal must be a JSON object"))?;
    reject_unknown(object.keys(), &["name", "token", "scopes"])
        .map_err(|_| invalid_at(index, "principal has an unknown field"))?;

    let name = object
        .get("name")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid_at(index, "principal `name` must be a string"))?;
    validate_name(name).map_err(|message| invalid_at(index, message))?;

    let token = object
        .get("token")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid_at(index, "principal `token` must be a string"))?;
    validate_token(token)?;

    let scopes = object
        .get("scopes")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| invalid_at(index, "principal `scopes` must be an array"))?;
    if scopes.is_empty() {
        return Err(invalid_at(index, "principal `scopes` must not be empty"));
    }
    let mut granted = BTreeSet::new();
    for scope in scopes {
        let text = scope
            .as_str()
            .ok_or_else(|| invalid_at(index, "principal `scopes` entries must be strings"))?;
        let scope = Scope::from_wire(text)
            .ok_or_else(|| invalid_at(index, "principal has an unknown scope"))?;
        if !granted.insert(scope) {
            return Err(invalid_at(index, "principal has a duplicate scope"));
        }
    }
    Ok((token.to_owned(), Principal::new(name, granted)))
}

fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > MAX_PRINCIPAL_NAME_BYTES {
        return Err("principal `name` must be 1..=64 bytes");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("principal `name` must use only ASCII letters, digits, `.`, `_` and `-`");
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<(), ServiceError> {
    if token.len() < MIN_TOKEN_BYTES || token.len() > MAX_TOKEN_BYTES {
        return Err(invalid("principal `token` must be 16..=256 bytes"));
    }
    if !token.bytes().all(|byte| (0x21..=0x7E).contains(&byte)) {
        return Err(invalid(
            "principal `token` must be printable ASCII without whitespace",
        ));
    }
    Ok(())
}

fn reject_unknown<'a>(
    keys: impl Iterator<Item = &'a String>,
    allowed: &[&str],
) -> Result<(), ServiceError> {
    for key in keys {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid("token file contains an unknown field"));
        }
    }
    Ok(())
}

fn invalid(message: &'static str) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

fn invalid_owned(message: String) -> ServiceError {
    ServiceError::new(ErrorCode::InvalidRequest, message)
}

fn invalid_at(index: usize, message: &'static str) -> ServiceError {
    ServiceError::new(
        ErrorCode::InvalidRequest,
        format!("principal {index}: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_PRINCIPAL_NAME_BYTES, Scope, TokenTable, parse_token_file};
    use crate::error::ErrorCode;

    const OPERATOR_TOKEN: &str = "operator-token-0001";
    const ADMIN_TOKEN: &str = "admin-token-00000002";

    fn document(principals: &str) -> String {
        format!("{{\"schema\":\"chronofork.tokens.v1\",\"principals\":[{principals}]}}")
    }

    fn valid_document() -> String {
        document(&format!(
            "{{\"name\":\"operator\",\"token\":\"{OPERATOR_TOKEN}\",\
             \"scopes\":[\"read\",\"write\"]}},\
             {{\"name\":\"admin\",\"token\":\"{ADMIN_TOKEN}\",\"scopes\":[\"admin\"]}}"
        ))
    }

    fn rejection(text: &str) -> ErrorCode {
        parse_token_file(text.as_bytes())
            .expect_err("token file must be rejected")
            .code()
    }

    #[test]
    fn a_valid_file_parses_and_authenticates() {
        let table = parse_token_file(valid_document().as_bytes()).expect("valid token file");
        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        let operator = table.authenticate(OPERATOR_TOKEN).expect("operator token");
        assert_eq!(operator.name(), "operator");
        assert!(operator.has_scope(Scope::Read));
        assert!(operator.has_scope(Scope::Write));
        assert!(!operator.has_scope(Scope::Admin));
        assert!(!operator.is_admin());
        assert!(table.authenticate("unknown-token-000000").is_none());
        assert_eq!(
            table.principal("admin").expect("admin principal").name(),
            "admin"
        );
        assert!(table.principal("missing").is_none());
    }

    #[test]
    fn admin_implies_read_and_write() {
        let table = parse_token_file(valid_document().as_bytes()).expect("valid token file");
        let admin = table.authenticate(ADMIN_TOKEN).expect("admin token");
        assert!(admin.is_admin());
        assert!(admin.has_scope(Scope::Read));
        assert!(admin.has_scope(Scope::Write));
        assert!(admin.has_scope(Scope::Admin));
    }

    #[test]
    fn scope_implies_nothing_for_an_unknown_principal_name() {
        let principal = super::Principal::new("reader", [Scope::Read]);
        assert!(principal.has_scope(Scope::Read));
        assert!(!principal.has_scope(Scope::Write));
        assert!(!principal.is_admin());
    }

    #[test]
    fn the_validation_matrix_is_typed() {
        // Empty, malformed and oversized documents.
        assert_eq!(rejection(""), ErrorCode::InvalidRequest);
        assert_eq!(rejection("[]"), ErrorCode::InvalidRequest);
        assert_eq!(
            rejection("{\"schema\":\"chronofork.tokens.v1\"}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&valid_document().replace("chronofork.tokens.v1", "x")),
            ErrorCode::InvalidRequest
        );

        // Empty table.
        assert_eq!(rejection(&document("")), ErrorCode::InvalidRequest);

        // Unknown fields at the root and inside a principal.
        assert_eq!(
            rejection("{\"schema\":\"chronofork.tokens.v1\",\"principals\":[],\"x\":1}"),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\"],\"role\":1}"
            )),
            ErrorCode::InvalidRequest
        );

        // Names.
        assert_eq!(
            rejection(&document(
                "{\"name\":\"\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );
        let long_name = "a".repeat(MAX_PRINCIPAL_NAME_BYTES + 1);
        assert_eq!(
            rejection(&document(&format!(
                "{{\"name\":\"{long_name}\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\"]}}"
            ))),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"bad name\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"bad/name\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );

        // Tokens.
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"short\",\"scopes\":[\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );
        let long_token = "t".repeat(257);
        assert_eq!(
            rejection(&document(&format!(
                "{{\"name\":\"operator\",\"token\":\"{long_token}\",\"scopes\":[\"read\"]}}"
            ))),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789 abcdef\",\"scopes\":[\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );

        // Scopes.
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789abcdef\",\"scopes\":[]}"
            )),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789abcdef\",\"scopes\":[\"root\"]}"
            )),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789abcdef\",\"scopes\":[\"read\",\"read\"]}"
            )),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(
                "{\"name\":\"operator\",\"token\":\"0123456789abcdef\",\"scopes\":\"read\"}"
            )),
            ErrorCode::InvalidRequest
        );

        // Duplicates.
        assert_eq!(
            rejection(&document(&format!(
                "{{\"name\":\"operator\",\"token\":\"{OPERATOR_TOKEN}\",\"scopes\":[\"read\"]}},\
                 {{\"name\":\"operator\",\"token\":\"{ADMIN_TOKEN}\",\"scopes\":[\"admin\"]}}"
            ))),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            rejection(&document(&format!(
                "{{\"name\":\"operator\",\"token\":\"{OPERATOR_TOKEN}\",\"scopes\":[\"read\"]}},\
                 {{\"name\":\"admin\",\"token\":\"{OPERATOR_TOKEN}\",\"scopes\":[\"admin\"]}}"
            ))),
            ErrorCode::InvalidRequest
        );

        // A principal that is not an object.
        assert_eq!(rejection(&document("1")), ErrorCode::InvalidRequest);
    }

    #[test]
    fn debug_output_names_principals_but_never_renders_tokens() {
        let table = parse_token_file(valid_document().as_bytes()).expect("valid token file");
        let rendered = format!("{table:?}");
        assert!(
            rendered.contains("operator") && rendered.contains("admin"),
            "principal names stay visible for diagnostics: {rendered}"
        );
        for (_, token) in table.credentials() {
            assert!(
                !rendered.contains(token),
                "token material leaked into `Debug`: {rendered}"
            );
        }
    }

    #[test]
    fn oversized_and_non_utf8_files_are_typed() {
        let oversized = format!("//{}", "x".repeat(1024 * 1024));
        assert_eq!(rejection(&oversized), ErrorCode::InvalidRequest);
        assert_eq!(
            parse_token_file(&[b'{', 0xFF, b'}'])
                .expect_err("invalid utf8")
                .code(),
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn credentials_yield_each_validated_pair_once_in_file_order() {
        let table = parse_token_file(valid_document().as_bytes()).expect("valid token file");
        let pairs: Vec<(&str, &str)> = table.credentials().collect();
        assert_eq!(
            pairs,
            vec![("operator", OPERATOR_TOKEN), ("admin", ADMIN_TOKEN)]
        );
        assert_eq!(pairs.len(), table.len());
        for (name, token) in table.credentials() {
            let principal = table.authenticate(token).expect("token authenticates");
            assert_eq!(principal.name(), name);
        }
    }

    #[test]
    fn an_empty_table_reports_empty() {
        let table = TokenTable::default();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
        assert!(table.authenticate(OPERATOR_TOKEN).is_none());
        assert!(table.principal("operator").is_none());
    }
}
