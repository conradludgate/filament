//! Connector error types and constants
//!
//! This module provides the error type used for connection operations
//! and the ALPN protocol identifier.

/// ALPN protocol identifier for Paxos connections
pub const PAXOS_ALPN: &[u8] = b"universal-sync/paxos/1";

/// Error type for connector operations
#[derive(Debug)]
pub enum ConnectorError {
    /// Connection failed
    Connect(String),
    /// Serialization/deserialization error
    Codec(String),
    /// IO error
    Io(std::io::Error),
    /// Handshake failed
    Handshake(String),
}

impl std::fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectorError::Connect(e) => write!(f, "connection error: {e}"),
            ConnectorError::Codec(e) => write!(f, "codec error: {e}"),
            ConnectorError::Io(e) => write!(f, "io error: {e}"),
            ConnectorError::Handshake(e) => write!(f, "handshake error: {e}"),
        }
    }
}

impl std::error::Error for ConnectorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectorError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConnectorError {
    fn from(e: std::io::Error) -> Self {
        ConnectorError::Io(e)
    }
}
