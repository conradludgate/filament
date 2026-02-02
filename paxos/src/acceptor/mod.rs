//! Acceptor runtime implementation
//!
//! This module provides the runtime components for running a Paxos acceptor:
//!
//! - [`AcceptorHandler`]: Handles individual protocol messages
//! - [`SharedAcceptorState`]: Thread-safe shared state for multi-connection acceptors
//! - [`run_acceptor`]: Main acceptor loop that processes connections
//!
//! # Example
//!
//! ```ignore
//! use paxos::acceptor::{AcceptorHandler, SharedAcceptorState, run_acceptor};
//!
//! let state = SharedAcceptorState::new();
//! let handler = AcceptorHandler::new(my_acceptor, state.clone());
//! run_acceptor(handler, connection, proposer_id).await?;
//! ```

mod handler;
mod runner;
mod state;

pub use handler::{AcceptError, AcceptOutcome, AcceptorHandler, InvalidProposal, PromiseOutcome};
pub use runner::run_acceptor;
pub use state::{AcceptorReceiver, AcceptorSubscription, RoundState, SharedAcceptorState};
