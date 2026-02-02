//! Actor management for proposer connections
//!
//! Uses a persistent actor model where each acceptor connection runs as an
//! independent spawned task, managed via `JoinMap`. Actors persist across rounds.

use std::collections::HashSet;
use std::pin::{Pin, pin};

use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio_util::task::JoinMap;
use tracing::{debug, instrument, trace, warn};

use crate::connection::LazyConnection;
use crate::messages::{AcceptorMessage, AcceptorRequest};
use crate::traits::{Connector, Learner, Proposal};

// ============================================================================
// Shared State Types
// ============================================================================

/// Command sent from coordinator to actors
pub(super) enum Command<L: Learner> {
    /// Idle - wait for next command
    Idle,
    /// Start prepare phase for this proposal
    Prepare(L::Proposal),
    /// Accept this proposal + message (after successful prepare)
    Accept {
        proposal: L::Proposal,
        message: L::Message,
    },
}

impl<L: Learner> Clone for Command<L> {
    fn clone(&self) -> Self {
        match self {
            Self::Idle => Self::Idle,
            Self::Prepare(p) => Self::Prepare(p.clone()),
            Self::Accept { proposal, message } => Self::Accept {
                proposal: proposal.clone(),
                message: message.clone(),
            },
        }
    }
}

#[expect(
    clippy::derivable_impls,
    reason = "derive(Default) doesn't work with generic bounds"
)]
impl<L: Learner> Default for Command<L> {
    fn default() -> Self {
        Self::Idle
    }
}

/// Messages from actors back to coordinator
pub(super) struct ActorMessage<L: Learner> {
    /// Which acceptor sent this message
    pub acceptor_id: <L::Proposal as Proposal>::NodeId,
    /// Sequence number for this proposal attempt
    pub seq: u64,
    /// The actual message
    pub msg: AcceptorMessage<L>,
}

/// Shared state broadcast from coordinator to all actors
pub(super) struct CoordinatorState<L: Learner> {
    /// Current command
    pub command: Command<L>,
    /// Sequence number - increments on each state change
    pub seq: u64,
}

impl<L: Learner> Clone for CoordinatorState<L> {
    fn clone(&self) -> Self {
        Self {
            command: self.command.clone(),
            seq: self.seq,
        }
    }
}

// ============================================================================
// Actor Implementation
// ============================================================================

/// Receive a message and forward it to the coordinator.
///
/// Returns `Some(msg)` if received, `None` if connection closed.
async fn recv_and_forward<L: Learner, C: Connector<L>>(
    mut conn: Pin<&mut LazyConnection<L, C>>,
    msg_tx: &mpsc::UnboundedSender<ActorMessage<L>>,
    acceptor_id: <L::Proposal as Proposal>::NodeId,
    seq: u64,
) -> Option<AcceptorMessage<L>> {
    let msg = conn.next().await?.ok()?;
    let _ = msg_tx.send(ActorMessage {
        acceptor_id,
        seq,
        msg: msg.clone(),
    });
    Some(msg)
}

/// Run an actor for a single acceptor connection.
///
/// This task lives as long as the acceptor is in the active set.
/// It handles connection lifecycle and responds to coordinator commands.
#[instrument(skip_all, name = "actor", fields(node_id = ?proposer_id, acceptor = ?acceptor_id))]
async fn run_actor<L, C>(
    proposer_id: <L::Proposal as Proposal>::NodeId,
    acceptor_id: <L::Proposal as Proposal>::NodeId,
    connector: C,
    mut state_rx: watch::Receiver<CoordinatorState<L>>,
    msg_tx: mpsc::UnboundedSender<ActorMessage<L>>,
) where
    L: Learner,
    C: Connector<L>,
{
    let mut conn = pin!(LazyConnection::new(acceptor_id, connector));
    let mut last_seq = 0u64;

    trace!("actor started");

    loop {
        // Wait for state change
        let state = {
            let Ok(state) = state_rx.wait_for(|s| s.seq != last_seq).await else {
                trace!("actor stopping: coordinator dropped");
                return;
            };
            last_seq = state.seq;
            trace!(seq = last_seq, "actor received state change");
            state.clone()
        };

        match state.command {
            Command::Prepare(proposal) => {
                let proposal_key = proposal.key();
                trace!("actor sending prepare");
                // Send prepare
                if conn.send(AcceptorRequest::Prepare(proposal)).await.is_err() {
                    warn!("actor prepare send failed, retrying on next command");
                    continue;
                }

                // Wait for a valid promise for THIS proposal.
                // Forward ALL messages to coordinator (for learning historical values).
                loop {
                    let Some(msg) =
                        recv_and_forward(conn.as_mut(), &msg_tx, acceptor_id, last_seq).await
                    else {
                        warn!("actor connection closed while waiting for promise");
                        break;
                    };
                    if msg.promised.key() >= proposal_key {
                        trace!("actor received promise for current proposal");
                        break;
                    }
                    trace!("actor received historical value, forwarded to coordinator");
                }

                // Wait for Accept command while also forwarding any connection messages
                // (for learners who need to receive live broadcasts)
                'accept_wait: loop {
                    tokio::select! {
                        biased;
                        // Check for new commands
                        result = state_rx.changed() => {
                            if result.is_err() {
                                trace!("actor stopping: coordinator dropped during accept_wait");
                                return;
                            }
                            let new_state = state_rx.borrow_and_update().clone();

                            // New round or new seq means restart
                            if new_state.seq != last_seq {
                                trace!(
                                    old_seq = last_seq,
                                    new_seq = new_state.seq,
                                    "actor restarting: seq changed"
                                );
                                break 'accept_wait;
                            }

                            if let Command::Accept {
                                proposal,
                                message: msg_value,
                            } = new_state.command
                            {
                                let proposal_key = proposal.key();
                                trace!("actor sending accept");

                                // Send accept
                                if conn
                                    .send(AcceptorRequest::Accept(proposal, msg_value))
                                    .await
                                    .is_err()
                                {
                                    warn!("actor accept send failed");
                                    break 'accept_wait;
                                }

                                // Wait for accepted response (also forward any other messages)
                                loop {
                                    let Some(msg) = recv_and_forward(conn.as_mut(), &msg_tx, acceptor_id, last_seq).await else {
                                        warn!("actor connection closed while waiting for accepted");
                                        break 'accept_wait;
                                    };
                                    if msg.accepted.as_ref().is_some_and(|(p, _)| p.key() >= proposal_key) {
                                        trace!("actor received accepted");
                                        break 'accept_wait;
                                    }
                                    trace!("actor received non-accepted message, forwarded");
                                }
                            }
                        }
                        // Also read from connection and forward any messages (for learners)
                        msg = recv_and_forward(conn.as_mut(), &msg_tx, acceptor_id, last_seq) => {
                            if msg.is_some() {
                                trace!("actor forwarded broadcast message");
                            } else {
                                warn!("actor connection closed during accept_wait");
                                break 'accept_wait;
                            }
                        }
                    }
                }
            }
            Command::Idle | Command::Accept { .. } => {
                trace!("actor received idle/accept command, ignoring");
            }
        }
    }
}

// ============================================================================
// Actor Manager
// ============================================================================

/// Manages the set of actor tasks
pub(super) struct ActorManager<L: Learner, C: Connector<L>> {
    proposer_id: <L::Proposal as Proposal>::NodeId,
    actors: JoinMap<<L::Proposal as Proposal>::NodeId, ()>,
    connector: C,
    state_tx: watch::Sender<CoordinatorState<L>>,
    state_rx: watch::Receiver<CoordinatorState<L>>,
    msg_tx: mpsc::UnboundedSender<ActorMessage<L>>,
}

impl<L: Learner, C: Connector<L>> Drop for ActorManager<L, C> {
    fn drop(&mut self) {
        // Abort all actors when manager is dropped to release connections
        self.actors.abort_all();
    }
}

impl<L, C> ActorManager<L, C>
where
    L: Learner,
    C: Connector<L>,
{
    pub fn new(
        proposer_id: <L::Proposal as Proposal>::NodeId,
        connector: C,
        msg_tx: mpsc::UnboundedSender<ActorMessage<L>>,
    ) -> Self {
        debug!("creating actor manager");
        let (state_tx, state_rx) = watch::channel(CoordinatorState {
            command: Command::Idle,
            seq: 0,
        });

        Self {
            proposer_id,
            actors: JoinMap::new(),
            connector,
            state_tx,
            state_rx,
            msg_tx,
        }
    }

    /// Update the actor set to match the current acceptors
    pub fn sync_actors(
        &mut self,
        acceptors: impl IntoIterator<Item = <L::Proposal as Proposal>::NodeId>,
    ) {
        let desired: HashSet<_> = acceptors.into_iter().collect();

        // Remove actors no longer in the set
        let before = self.actors.len();
        self.actors
            .abort_matching(|node_id| !desired.contains(node_id));
        let removed = before - self.actors.len();

        // Spawn new actors
        let mut spawned = 0;
        for id in desired {
            if !self.actors.contains_key(&id) {
                let proposer_id = self.proposer_id;
                let connector = self.connector.clone();
                let state_rx = self.state_rx.clone();
                let msg_tx = self.msg_tx.clone();
                self.actors
                    .spawn(id, run_actor(proposer_id, id, connector, state_rx, msg_tx));
                spawned += 1;
            }
        }
        if removed > 0 || spawned > 0 {
            debug!(removed, spawned, total = self.actors.len(), "synced actors");
        }
    }

    pub fn num_actors(&self) -> usize {
        self.actors.len()
    }

    /// Start a new proposal (Prepare phase). Increments seq to signal new proposal.
    pub fn start_prepare(&self, proposal: L::Proposal) {
        self.state_tx.send_modify(|s| {
            s.command = Command::Prepare(proposal);
            s.seq += 1;
        });
        debug!(seq = self.state_tx.borrow().seq, "started prepare phase");
    }

    /// Transition to Accept phase. Does NOT increment seq so actors don't restart.
    pub fn transition_to_accept(&self, proposal: L::Proposal, message: L::Message) {
        self.state_tx.send_modify(|s| {
            s.command = Command::Accept { proposal, message };
        });
        debug!("transitioned to accept phase");
    }

    pub fn current_seq(&self) -> u64 {
        self.state_tx.borrow().seq
    }
}
