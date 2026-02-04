//! Universal Sync Proposer - client/device-side group membership
//!
//! This crate provides the proposer (client/device) implementation for
//! Universal Sync, including:
//!
//! - [`GroupClient`] - High-level client abstraction for creating/joining groups
//! - [`Group`] - High-level API for synchronized MLS groups
//! - [`GroupLearner`] - Lower-level MLS group member (for advanced use)
//! - [`IrohConnector`] - P2P QUIC connections to acceptors

#![warn(clippy::pedantic)]

pub mod client;
pub mod connection;
pub mod connector;
pub mod error;
pub mod flows;
pub mod group;
pub mod group_state;
pub mod learner;
pub mod rendezvous;

/// REPL module for the CLI binary. Not part of the public API.
#[doc(hidden)]
pub mod repl;

pub use client::GroupClient;
pub use connection::ConnectionManager;
pub use connector::{
    ConnectorError, IrohConnection, IrohConnector, PAXOS_ALPN, register_group,
    register_group_with_addr,
};
pub use error::GroupError;
pub use flows::acceptors_extension;
pub use group::{Group, GroupContext, GroupEvent, ReceivedAppMessage, wait_for_welcome};
pub use group_state::{FjallGroupStateStorage, GroupStateError};
pub use learner::{GroupLearner, LearnerError};
// Re-export core types for convenience
pub use universal_sync_core::{
    ACCEPTOR_ADD_EXTENSION_TYPE, ACCEPTOR_REMOVE_EXTENSION_TYPE, ACCEPTORS_EXTENSION_TYPE,
    AcceptorAdd, AcceptorId, AcceptorRemove, AcceptorsExt, Attempt, Epoch, GroupId, GroupMessage,
    GroupProposal, Handshake, HandshakeResponse, MEMBER_ADDR_EXTENSION_TYPE, MemberAddrExt,
    MemberId, UnsignedProposal,
};
