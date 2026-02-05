//! Common error types and context structures for Universal Sync
//!
//! This module provides:
//! - [`ConnectorError`]: Errors for network connection operations
//! - Structured context types for rich error attribution with `error_stack`
//!
//! # Using Structured Contexts
//!
//! Instead of string formatting, use structured context types for better debugging:
//!
//! ```ignore
//! use error_stack::{Report, ResultExt};
//! use universal_sync_core::error::{GroupContext, AcceptorContext};
//!
//! fn process_proposal(group_id: GroupId, acceptor_id: AcceptorId) -> Result<(), Report<MyError>> {
//!     do_something()
//!         .change_context(MyError)
//!         .attach(GroupContext::new(group_id))
//!         .attach(AcceptorContext::new(acceptor_id))?;
//!     Ok(())
//! }
//! ```

use std::fmt;

use crate::{AcceptorId, Epoch, GroupId, MemberId};

// =============================================================================
// Connector Error
// =============================================================================

/// Error type for network connection operations.
///
/// Used by both proposer and acceptor connectors.
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

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectorError::Connect(e) => write!(f, "connection failed: {e}"),
            ConnectorError::Codec(e) => write!(f, "codec error: {e}"),
            ConnectorError::Io(e) => write!(f, "IO error: {e}"),
            ConnectorError::Handshake(e) => write!(f, "handshake failed: {e}"),
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

impl From<ConnectorError> for std::io::Error {
    fn from(e: ConnectorError) -> Self {
        match e {
            ConnectorError::Io(io_err) => io_err,
            other => std::io::Error::other(other),
        }
    }
}

// =============================================================================
// Structured Error Contexts
// =============================================================================

/// Context: which group an error occurred in.
///
/// Attach to errors when the group ID is relevant for debugging.
#[derive(Debug, Clone)]
pub struct GroupContext {
    /// The group ID
    pub group_id: GroupId,
}

impl GroupContext {
    /// Create a new group context.
    #[must_use]
    pub fn new(group_id: GroupId) -> Self {
        Self { group_id }
    }
}

impl fmt::Display for GroupContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "group: {}",
            bs58::encode(&self.group_id.0[..8]).into_string()
        )
    }
}

/// Context: which acceptor an error occurred with.
///
/// Attach to errors when the acceptor ID is relevant for debugging.
#[derive(Debug, Clone)]
pub struct AcceptorContext {
    /// The acceptor ID
    pub acceptor_id: AcceptorId,
}

impl AcceptorContext {
    /// Create a new acceptor context.
    #[must_use]
    pub fn new(acceptor_id: AcceptorId) -> Self {
        Self { acceptor_id }
    }
}

impl fmt::Display for AcceptorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "acceptor: {}",
            bs58::encode(&self.acceptor_id.0[..8]).into_string()
        )
    }
}

/// Context: which epoch an error occurred in.
///
/// Attach to errors when the MLS epoch is relevant for debugging.
#[derive(Debug, Clone, Copy)]
pub struct EpochContext {
    /// The epoch number
    pub epoch: Epoch,
}

impl EpochContext {
    /// Create a new epoch context.
    #[must_use]
    pub fn new(epoch: Epoch) -> Self {
        Self { epoch }
    }
}

impl fmt::Display for EpochContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch: {}", self.epoch.0)
    }
}

/// Context: which member an error is related to.
///
/// Attach to errors when the member ID/index is relevant for debugging.
#[derive(Debug, Clone, Copy)]
pub struct MemberContext {
    /// The member ID (index in the MLS roster)
    pub member_id: MemberId,
}

impl MemberContext {
    /// Create a new member context.
    #[must_use]
    pub fn new(member_id: MemberId) -> Self {
        Self { member_id }
    }
}

impl fmt::Display for MemberContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "member index: {}", self.member_id.0)
    }
}

/// Context: operation that was being performed when error occurred.
///
/// Provides high-level context about what the code was trying to do.
#[derive(Debug, Clone)]
pub struct OperationContext {
    /// Description of the operation
    pub operation: &'static str,
}

impl OperationContext {
    /// Create a new operation context.
    #[must_use]
    pub fn new(operation: &'static str) -> Self {
        Self { operation }
    }
}

impl fmt::Display for OperationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "while {}", self.operation)
    }
}

// Common operations as constants for consistency
impl OperationContext {
    /// Creating a new group
    pub const CREATING_GROUP: Self = Self {
        operation: "creating group",
    };
    /// Joining an existing group
    pub const JOINING_GROUP: Self = Self {
        operation: "joining group",
    };
    /// Adding a member
    pub const ADDING_MEMBER: Self = Self {
        operation: "adding member",
    };
    /// Removing a member
    pub const REMOVING_MEMBER: Self = Self {
        operation: "removing member",
    };
    /// Adding an acceptor
    pub const ADDING_ACCEPTOR: Self = Self {
        operation: "adding acceptor",
    };
    /// Removing an acceptor
    pub const REMOVING_ACCEPTOR: Self = Self {
        operation: "removing acceptor",
    };
    /// Proposing via Paxos
    pub const PROPOSING: Self = Self {
        operation: "proposing via Paxos",
    };
    /// Connecting to acceptor
    pub const CONNECTING: Self = Self {
        operation: "connecting to acceptor",
    };
    /// Validating proposal
    pub const VALIDATING_PROPOSAL: Self = Self {
        operation: "validating proposal",
    };
    /// Applying learned value
    pub const APPLYING_VALUE: Self = Self {
        operation: "applying learned value",
    };
    /// Sending welcome message
    pub const SENDING_WELCOME: Self = Self {
        operation: "sending welcome message",
    };
    /// Processing MLS message
    pub const PROCESSING_MLS: Self = Self {
        operation: "processing MLS message",
    };
}
