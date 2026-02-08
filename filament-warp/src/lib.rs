//! Multi-Paxos consensus library with signed proposals.

#![warn(clippy::pedantic)]

use std::fmt;
use std::hash::Hash;

use error_stack::Report;
use futures::{Sink, Stream};

pub mod acceptor;
pub mod core;
pub mod proposer;

pub use acceptor::{AcceptorMessage, AcceptorRequest, AcceptorStateStore};
pub use proposer::{ProposeResult, Proposer};

#[derive(Debug)]
pub struct ValidationError;

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("proposal validation failed")
    }
}

impl std::error::Error for ValidationError {}

/// Marker type proving that validation was performed.
/// Cannot be constructed outside of validation functions.
#[derive(Debug, Clone, Copy)]
pub struct Validated(());

impl Validated {
    /// Only call this after actually performing all validation checks.
    #[must_use]
    pub fn assert_valid() -> Self {
        Self(())
    }
}

pub trait Proposal: Clone {
    type NodeId: Copy + Ord + fmt::Debug + Hash + Send + Sync;
    type RoundId: Copy + Ord + Default + fmt::Debug + Hash + Send + Sync;
    type AttemptId: Copy + Ord + Default + fmt::Debug + Hash + Send + Sync;

    fn node_id(&self) -> Self::NodeId;
    fn round(&self) -> Self::RoundId;
    fn attempt(&self) -> Self::AttemptId;
    fn next_attempt(attempt: Self::AttemptId) -> Self::AttemptId;

    fn key(&self) -> ProposalKey<Self> {
        ProposalKey::new(self.round(), self.attempt(), self.node_id())
    }
}

/// Ordering key for proposals — compares by (round, attempt, `node_id`).
#[derive(Debug)]
pub struct ProposalKey<P: Proposal>(
    pub(crate) P::RoundId,
    pub(crate) P::AttemptId,
    pub(crate) P::NodeId,
);

impl<P: Proposal> Copy for ProposalKey<P> {}

#[expect(clippy::expl_impl_clone_on_copy)]
impl<P: Proposal> Clone for ProposalKey<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: Proposal> Hash for ProposalKey<P> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let ProposalKey(round, attempt, node_id) = self;
        round.hash(state);
        attempt.hash(state);
        node_id.hash(state);
    }
}

impl<P: Proposal> Eq for ProposalKey<P> {}

impl<P: Proposal> PartialEq for ProposalKey<P> {
    fn eq(&self, other: &Self) -> bool {
        let ProposalKey(round1, attempt1, node1) = self;
        let ProposalKey(round2, attempt2, node2) = other;
        round1 == round2 && attempt1 == attempt2 && node1 == node2
    }
}

impl<P: Proposal> PartialOrd for ProposalKey<P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: Proposal> Ord for ProposalKey<P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let ProposalKey(round1, attempt1, node1) = self;
        let ProposalKey(round2, attempt2, node2) = other;
        (round1.cmp(round2))
            .then(attempt1.cmp(attempt2))
            .then(node1.cmp(node2))
    }
}

impl<P: Proposal> ProposalKey<P> {
    #[must_use]
    pub(crate) fn new(round: P::RoundId, attempt: P::AttemptId, node_id: P::NodeId) -> Self {
        Self(round, attempt, node_id)
    }

    #[must_use]
    pub fn attempt(&self) -> P::AttemptId {
        self.1
    }
}

/// State machine that learns from consensus and can create proposals.
///
/// For devices/clients, `propose()` creates a signed proposal with real content.
/// For acceptors, `propose()` creates a sync-only proposal for the learning process.
#[expect(async_fn_in_trait)]
pub trait Learner: Send + Sync + 'static {
    type Proposal: Proposal + fmt::Debug + Send + Sync + 'static;
    type Message: Clone + fmt::Debug + Send + Sync + 'static;
    type Error: fmt::Debug + Send + 'static;
    type AcceptorId: Copy + Ord + fmt::Debug + Hash + Send + Sync;

    fn node_id(&self) -> <Self::Proposal as Proposal>::NodeId;
    fn current_round(&self) -> <Self::Proposal as Proposal>::RoundId;
    fn acceptors(&self) -> impl IntoIterator<Item = Self::AcceptorId, IntoIter: ExactSizeIterator>;
    fn propose(&self, attempt: <Self::Proposal as Proposal>::AttemptId) -> Self::Proposal;
    /// # Errors
    ///
    /// Returns [`ValidationError`] if the proposal is invalid for the current state.
    fn validate(&self, proposal: &Self::Proposal) -> Result<Validated, Report<ValidationError>>;
    async fn apply(
        &mut self,
        proposal: Self::Proposal,
        message: Self::Message,
    ) -> Result<(), Self::Error>;
}

pub trait AcceptorConn<L: Learner>:
    Sink<AcceptorRequest<L>, Error = L::Error> + Stream<Item = Result<AcceptorMessage<L>, L::Error>>
{
}

impl<L, T> AcceptorConn<L> for T
where
    L: Learner,
    T: Sink<AcceptorRequest<L>, Error = L::Error>
        + Stream<Item = Result<AcceptorMessage<L>, L::Error>>,
{
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    #[test]
    fn validation_error_display() {
        assert_eq!(ValidationError.to_string(), "proposal validation failed");
        let _: &dyn std::error::Error = &ValidationError;
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct P {
        node: u32,
        round: u64,
        attempt: u32,
    }

    impl Proposal for P {
        type NodeId = u32;
        type RoundId = u64;
        type AttemptId = u32;
        fn node_id(&self) -> u32 {
            self.node
        }
        fn round(&self) -> u64 {
            self.round
        }
        fn attempt(&self) -> u32 {
            self.attempt
        }
        fn next_attempt(a: u32) -> u32 {
            a + 1
        }
    }

    fn hash_of(k: &ProposalKey<P>) -> u64 {
        use std::hash::Hasher;
        let mut h = DefaultHasher::new();
        k.hash(&mut h);
        h.finish()
    }

    #[test]
    fn proposal_key_hash_eq() {
        let k1 = ProposalKey::<P>::new(1, 2, 3);
        let k2 = ProposalKey::<P>::new(1, 2, 3);
        assert_eq!(hash_of(&k1), hash_of(&k2));
    }

    #[test]
    fn proposal_key_hash_ne() {
        let k1 = ProposalKey::<P>::new(1, 2, 3);
        let k2 = ProposalKey::<P>::new(1, 2, 4);
        assert_ne!(hash_of(&k1), hash_of(&k2));
    }
}
