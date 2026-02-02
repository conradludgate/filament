//! Connection handshake protocol
//!
//! When a client connects to an acceptor, it must first send a handshake
//! message to identify which group it wants to interact with.

use serde::{Deserialize, Serialize};

/// Group identifier (32 bytes)
///
/// MLS group IDs are hashed/padded to 32 bytes for consistent key sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub [u8; 32]);

impl GroupId {
    /// Create a new group ID from a 32-byte array
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Create a group ID from a slice, padding or truncating as needed
    ///
    /// - If shorter than 32 bytes, pads with zeros
    /// - If longer than 32 bytes, truncates
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut id = [0u8; 32];
        let len = bytes.len().min(32);
        id[..len].copy_from_slice(&bytes[..len]);
        Self(id)
    }

    /// Get the bytes of this group ID
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for GroupId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for GroupId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Initial handshake message sent by clients
///
/// This is the first message on any bidirectional stream. It tells
/// the acceptor which group the client wants to interact with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Handshake {
    /// Join an existing group by ID
    ///
    /// The acceptor must already have this group's state.
    Join(GroupId),

    /// Create/join a new group with the provided GroupInfo
    ///
    /// The GroupInfo is an MLS message that allows external parties
    /// to join the group. This is used when first registering a group
    /// with an acceptor.
    ///
    /// The bytes are a serialized `MlsMessage` containing GroupInfo.
    Create(Vec<u8>),
}

/// Handshake response from the acceptor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandshakeResponse {
    /// Handshake accepted, proceed with Paxos protocol
    Ok,

    /// Group not found (for Join)
    GroupNotFound,

    /// Invalid GroupInfo (for Create)
    InvalidGroupInfo(String),

    /// Other error
    Error(String),
}
