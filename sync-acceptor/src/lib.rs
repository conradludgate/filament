//! Universal Sync Acceptor - server/federation-side group membership
//!
//! This crate provides the acceptor (server/federation) implementation for
//! Universal Sync, including:
//!
//! - [`GroupAcceptor`] - External group observer that validates proposals
//! - [`FjallStateStore`] / [`GroupStateStore`] - Persistent state storage
//! - [`AcceptorRegistry`] - Multi-group management
//! - Connection handling via iroh

#![warn(clippy::pedantic)]

pub(crate) mod acceptor;
pub(crate) mod connector;
pub(crate) mod epoch_roster;
pub(crate) mod learner;
pub(crate) mod registry;
pub(crate) mod server;
pub(crate) mod state_store;

pub use acceptor::{AcceptorError, GroupAcceptor};
pub use connector::ConnectorError;
pub use registry::AcceptorRegistry;
pub use server::{IrohAcceptorConnection, accept_connection};
pub use state_store::{GroupStateStore, SharedFjallStateStore};
