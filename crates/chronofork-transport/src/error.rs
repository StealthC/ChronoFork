//! Typed transport failures.

use std::fmt;

use chronofork_service::ServiceError;

/// A failure at the HTTP/MCP boundary.
#[derive(Debug)]
pub enum TransportError {
    /// A socket or stream operation failed.
    Io(std::io::Error),
    /// A base URL is not a supported `http://host:port` address.
    InvalidUrl(String),
    /// The peer violated the HTTP/1.1 or JSON-RPC framing rules.
    Protocol(String),
    /// The peer answered with a non-success status.
    Http {
        /// HTTP status code.
        status: u16,
        /// Canonical error or result body, when one was returned.
        body: Vec<u8>,
    },
    /// A service operation failed locally (in-process client paths).
    Service(ServiceError),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "transport I/O error: {error}"),
            Self::InvalidUrl(url) => write!(formatter, "invalid base URL `{url}`"),
            Self::Protocol(message) => write!(formatter, "transport protocol error: {message}"),
            Self::Http { status, .. } => write!(formatter, "peer returned HTTP status {status}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Service(error) => Some(error),
            Self::InvalidUrl(_) | Self::Protocol(_) | Self::Http { .. } => None,
        }
    }
}

impl From<std::io::Error> for TransportError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ServiceError> for TransportError {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}
