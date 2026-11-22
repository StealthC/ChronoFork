//! Bounded HTTP/1.1 and MCP Streamable HTTP adapters for ChronoFork.
//!
//! The transport layer is intentionally thin: it authenticates a bearer token, maps
//! bytes to the service catalog and enforces wire-level resource bounds. It owns no
//! session state and reimplements no service rule. See
//! `docs/adr/0012-agent-native-interfaces.md`.

#![cfg_attr(test, allow(clippy::expect_used))]

pub mod admission;
mod client;
pub mod error;
pub mod http;
pub mod mcp;
mod wire;

pub use client::{HttpClient, McpClient};
pub use error::TransportError;
pub use http::{ServerConfig, ServerHandle, serve};
pub use mcp::ToolCallResult;
