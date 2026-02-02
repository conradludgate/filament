//! Acceptor handler for processing Paxos protocol messages

use std::fmt;

use tracing::trace;

use crate::messages::AcceptorMessage;
use crate::traits::{Acceptor, AcceptorStateStore, Proposal};

/// Error returned when a proposal fails validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidProposal;

impl fmt::Display for InvalidProposal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid proposal")
    }
}

impl std::error::Error for InvalidProposal {}

/// Outcome of handling a Prepare request.
pub enum PromiseOutcome<A: Acceptor> {
    /// Successfully promised to this proposal.
    Promised(AcceptorMessage<A>),
    /// Rejected because a higher proposal was already promised/accepted.
    Outdated(AcceptorMessage<A>),
}

/// Outcome of handling an Accept request.
pub enum AcceptOutcome<A: Acceptor> {
    /// Successfully accepted this proposal.
    Accepted(AcceptorMessage<A>),
    /// Rejected because a higher proposal was already promised/accepted.
    Outdated(AcceptorMessage<A>),
}

/// Error returned when an Accept request fails.
pub enum AcceptError<E> {
    /// The proposal failed validation.
    InvalidProposal,
    /// Failed to persist the accepted value.
    PersistFailed(E),
}

impl<E: fmt::Debug> fmt::Debug for AcceptError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProposal => write!(f, "InvalidProposal"),
            Self::PersistFailed(e) => f.debug_tuple("PersistFailed").field(e).finish(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for AcceptError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProposal => write!(f, "invalid proposal"),
            Self::PersistFailed(e) => write!(f, "persist failed: {e}"),
        }
    }
}

impl<E: std::error::Error> std::error::Error for AcceptError<E> {}

/// Handler for acceptor requests.
///
/// Wraps an [`Acceptor`] and [`AcceptorStateStore`] to provide a clean API
/// for handling Paxos protocol messages.
pub struct AcceptorHandler<A, S> {
    acceptor: A,
    state: S,
}

impl<A, S> AcceptorHandler<A, S>
where
    A: Acceptor,
    S: AcceptorStateStore<A>,
{
    /// Create a new acceptor handler.
    pub fn new(acceptor: A, state: S) -> Self {
        Self { acceptor, state }
    }

    /// Get the node ID of this acceptor.
    pub fn node_id(&self) -> <A::Proposal as Proposal>::NodeId {
        self.acceptor.node_id()
    }

    /// Get a reference to the underlying state.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Handle a Prepare request.
    ///
    /// Returns `Ok(Promised(...))` if the promise succeeded.
    /// Returns `Ok(Outdated(...))` if already promised/accepted a higher proposal.
    ///
    /// # Errors
    ///
    /// Returns `Err(InvalidProposal)` if the proposal fails validation.
    pub fn handle_prepare(
        &mut self,
        proposal: &A::Proposal,
    ) -> Result<PromiseOutcome<A>, InvalidProposal> {
        if !self.acceptor.validate(proposal) {
            return Err(InvalidProposal);
        }

        match self.state.promise(proposal) {
            Ok(()) => {
                trace!(round = ?proposal.round(), "promised");
                let state = self.state.get(proposal.round());
                Ok(PromiseOutcome::Promised(AcceptorMessage::from_round_state(
                    state,
                )))
            }
            Err(round_state) => {
                trace!(round = ?proposal.round(), "promise rejected - outdated");
                Ok(PromiseOutcome::Outdated(AcceptorMessage::from_round_state(
                    round_state,
                )))
            }
        }
    }

    /// Handle an Accept request.
    ///
    /// Returns `Ok(Accepted(...))` if the accept succeeded.
    /// Returns `Ok(Outdated(...))` if already promised/accepted a higher proposal.
    ///
    /// # Errors
    ///
    /// - `Err(InvalidProposal)` if validation fails.
    /// - `Err(PersistFailed(...))` if the acceptor fails to persist.
    pub async fn handle_accept(
        &mut self,
        proposal: &A::Proposal,
        message: A::Message,
    ) -> Result<AcceptOutcome<A>, AcceptError<A::Error>> {
        if !self.acceptor.validate(proposal) {
            return Err(AcceptError::InvalidProposal);
        }

        match self.state.accept(proposal, &message) {
            Ok(()) => {
                trace!(round = ?proposal.round(), "accepted");
                self.acceptor
                    .accept(proposal.clone(), message)
                    .await
                    .map_err(AcceptError::PersistFailed)?;
                let state = self.state.get(proposal.round());
                Ok(AcceptOutcome::Accepted(AcceptorMessage::from_round_state(
                    state,
                )))
            }
            Err(round_state) => {
                trace!(round = ?proposal.round(), "accept rejected - outdated");
                Ok(AcceptOutcome::Outdated(AcceptorMessage::from_round_state(
                    round_state,
                )))
            }
        }
    }
}
