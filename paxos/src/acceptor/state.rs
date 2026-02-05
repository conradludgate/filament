//! Shared acceptor state implementation

use crate::traits::Learner;

/// Per-round acceptor state.
///
/// Contains the highest promised proposal and the accepted (proposal, message) pair.
pub struct RoundState<L: Learner> {
    /// Highest promised proposal
    pub promised: Option<L::Proposal>,
    /// Accepted (proposal, message) pair
    pub accepted: Option<(L::Proposal, L::Message)>,
}

impl<L: Learner> Clone for RoundState<L> {
    fn clone(&self) -> Self {
        Self {
            promised: self.promised.clone(),
            accepted: self.accepted.clone(),
        }
    }
}

impl<L: Learner> Default for RoundState<L> {
    fn default() -> Self {
        Self {
            promised: None,
            accepted: None,
        }
    }
}
