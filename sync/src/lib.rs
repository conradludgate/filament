//! Universal Sync - Group membership over Paxos with MLS
//!
//! This crate provides group membership management using:
//! - MLS (Messaging Layer Security) for cryptographic group state
//! - Paxos for consensus on group changes
//!
//! # Architecture
//!
//! - **Devices** are group members with full MLS `Group` state
//! - **Acceptors** are federated servers with `ExternalGroup` state (can verify, can't decrypt)

pub mod acceptor;
pub mod connector;
pub mod extension;
pub mod flows;
pub mod handshake;
pub mod learner;
pub mod message;
pub mod proposal;
pub mod registry;
pub mod server;
pub mod state_store;
pub mod testing;

pub use acceptor::GroupAcceptor;
pub use connector::{
    ConnectorError, IrohConnection, IrohConnector, PAXOS_ALPN, register_group,
    register_group_with_addr,
};
pub use extension::{
    ACCEPTOR_ADD_EXTENSION_TYPE, ACCEPTOR_REMOVE_EXTENSION_TYPE, ACCEPTORS_EXTENSION_TYPE,
    AcceptorAdd, AcceptorRemove, AcceptorsExt,
};
pub use flows::{
    CreatedGroup, FlowError, JoinedGroup, acceptors_extension, create_group,
    create_group_with_addrs, join_group,
};
pub use handshake::{GroupId, Handshake, HandshakeResponse};
pub use learner::GroupLearner;
pub use message::GroupMessage;
pub use proposal::{AcceptorId, GroupProposal, MemberId};
pub use registry::AcceptorRegistry;
pub use server::{GroupRegistry, IrohAcceptorConnection, accept_connection};
pub use state_store::{GroupStateStore, SharedFjallStateStore};
