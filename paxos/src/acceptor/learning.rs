//! Learning acceptor that tracks quorum and applies on consensus
//!
//! This module provides an [`AcceptorLearner`] that tracks accepted values
//! from multiple acceptors and determines when quorum is reached.
//!
//! # Architecture
//!
//! An acceptor server should run both:
//! 1. The acceptor protocol (via [`run_acceptor`](super::run_acceptor))
//! 2. A learner (via [`AcceptorLearner`]) that applies values on quorum
//!
//! # Usage
//!
//! ```ignore
//! use paxos::acceptor::{AcceptorLearner, SharedAcceptorState};
//!
//! // Create learner with subscriptions to all acceptor state stores
//! let mut learner = AcceptorLearner::new();
//!
//! // Sync with current acceptor set
//! learner.sync_acceptors(acceptor_states.iter().map(|s| s.subscribe_from(round)));
//!
//! // Learn values in a loop
//! while let Some((proposal, message)) = learner.learn_one(&my_learner).await {
//!     my_learner.apply(proposal, message).await?;
//! }
//! ```

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use pin_project_lite::pin_project;
use tracing::debug;

use crate::proposer::QuorumTracker;
use crate::traits::{Learner, Proposal};

pin_project! {
    /// A combined stream from multiple acceptor subscriptions
    struct CombinedStream<S> {
        #[pin]
        streams: Vec<S>,
    }
}

impl<S, P, M> Stream for CombinedStream<S>
where
    S: Stream<Item = (P, M)> + Unpin,
{
    type Item = (P, M);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        for stream in this.streams.iter_mut() {
            if let Poll::Ready(Some(item)) = Pin::new(stream).poll_next(cx) {
                return Poll::Ready(Some(item));
            }
        }

        // Check if all streams are done
        if this.streams.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

/// Learner that tracks quorum across multiple acceptors
///
/// Similar to [`Proposer`](crate::proposer::Proposer), this manages
/// subscriptions to acceptors and determines when values reach consensus.
///
/// Uses the same [`QuorumTracker`] as the proposer runtime for consistency.
///
/// # Dynamic Acceptor Sets
///
/// Use [`sync_acceptors`](Self::sync_acceptors) to update the set of acceptors
/// when the membership changes. The quorum is recalculated based on the new set.
pub struct AcceptorLearner<L: Learner, S> {
    /// Combined stream of all acceptor subscriptions
    streams: CombinedStream<S>,
    /// Quorum tracking (reuses proposer's `QuorumTracker`)
    tracker: QuorumTracker<L>,
}

impl<L, S> AcceptorLearner<L, S>
where
    L: Learner,
    L::Proposal: Clone,
    L::Message: Clone,
    S: Stream<Item = (L::Proposal, L::Message)> + Unpin,
{
    /// Create a new learner with no acceptors
    ///
    /// Call [`sync_acceptors`](Self::sync_acceptors) to add acceptor subscriptions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            streams: CombinedStream { streams: vec![] },
            tracker: QuorumTracker::new(0),
        }
    }

    /// Sync the acceptor set with new subscriptions
    ///
    /// Replaces all current subscriptions with the new set.
    /// The quorum is recalculated based on the new acceptor count.
    ///
    /// # Arguments
    ///
    /// * `subscriptions` - Iterator of subscription streams from each acceptor's state store
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Get subscriptions from all acceptor state stores
    /// let subscriptions = acceptor_states
    ///     .iter()
    ///     .map(|s| s.subscribe_from(current_round));
    ///
    /// learner.sync_acceptors(subscriptions);
    /// ```
    pub fn sync_acceptors(&mut self, subscriptions: impl IntoIterator<Item = S>) {
        let streams: Vec<S> = subscriptions.into_iter().collect();
        let count = streams.len();

        self.streams = CombinedStream { streams };
        self.tracker = QuorumTracker::new(count);

        debug!(
            acceptor_count = count,
            quorum = self.tracker.quorum(),
            "synced acceptors"
        );
    }

    /// Get the current acceptor count
    #[must_use]
    pub fn acceptor_count(&self) -> usize {
        self.tracker.num_acceptors()
    }

    /// Get the current quorum threshold
    #[must_use]
    pub fn quorum(&self) -> usize {
        self.tracker.quorum()
    }

    /// Learn one value that has reached quorum
    ///
    /// Waits until a value is confirmed by a quorum of acceptors.
    /// Returns `Some((proposal, message))` when quorum is reached,
    /// or `None` if all acceptor streams are closed.
    ///
    /// The caller is responsible for calling [`Learner::apply`] after
    /// receiving the result.
    ///
    /// # Cancellation Safety
    ///
    /// This method is cancellation-safe. Messages received before cancellation
    /// are buffered internally. If cancelled, calling `learn_one` again will
    /// first check for quorum from buffered messages.
    pub async fn learn_one(&mut self, learner: &L) -> Option<(L::Proposal, L::Message)> {
        let current_round = learner.current_round();

        // First check if we already have quorum from previously buffered messages
        if let Some((p, m)) = self.tracker.check_quorum(current_round) {
            debug!(?current_round, "learned from buffered messages");
            return Some((p.clone(), m.clone()));
        }

        debug!(
            ?current_round,
            quorum = self.tracker.quorum(),
            acceptors = self.tracker.num_acceptors(),
            "learning"
        );

        loop {
            let Some((proposal, message)) = self.streams.next().await else {
                debug!("all acceptor streams closed");
                return None;
            };

            // Skip rounds before current
            if proposal.round() < current_round {
                continue;
            }

            // Track and check for quorum
            if let Some((p, m)) = self.tracker.track(proposal, message) {
                debug!(round = ?p.round(), "learned with quorum");
                return Some((p.clone(), m.clone()));
            }
        }
    }
}

impl<L, S> Default for AcceptorLearner<L, S>
where
    L: Learner,
    L::Proposal: Clone,
    L::Message: Clone,
    S: Stream<Item = (L::Proposal, L::Message)> + Unpin,
{
    fn default() -> Self {
        Self::new()
    }
}
