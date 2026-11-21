//! Transport-agnostic application service for ChronoFork.
//!
//! This crate owns the typed operation catalog, canonical JSON, authentication and
//! ownership policy and the single synchronization boundary around
//! [`chronofork_sessions::SessionManager`]. It never contains a socket, HTTP header,
//! JSON-RPC frame or CLI type; `chronofork-transport` and the CLI are thin adapters
//! over this API. See `docs/adr/0012-agent-native-interfaces.md`.

#![cfg_attr(test, allow(clippy::expect_used))]

mod catalog;
mod durable;
mod experiment;
mod runtime;
mod values;
pub mod version;

pub mod auth;
pub mod error;
pub mod json;
pub mod limits;
pub mod service;
pub mod tool;

pub use auth::{Principal, Scope, TokenTable, parse_token_file};
pub use error::{ERROR_SCHEMA, ErrorCode, ServiceError, WIRE_VERSION};
pub use json::JsonValue;
pub use limits::ServiceLimits;
pub use service::{AppService, ServiceConfig};
pub use tool::ToolSpec;
pub use version::ApiVersion;
