//! Acceptor learning infrastructure
//!
//! This module provides the learning capability for acceptors in a multi-acceptor
//! deployment. Each acceptor acts as a learner, connecting to peer acceptors to:
//!
//! 1. Receive accepted values from peers
//! 2. Track quorum across all acceptors (including itself)
//! 3. Notify when quorum is reached so values can be applied
//!
//! # Architecture
//!
//! ```text
//!                    ┌─────────────────────────────────────┐
//!                    │         GroupLearningActor          │
//!                    │                                     │
//!  Local state ─────►│  ┌─────────────────────────────┐   │
//!  subscription      │  │      QuorumTracker          │   │
//!                    │  │  (tracks quorum per round)  │   │──► epoch notification
//!  Peer streams ────►│  └─────────────────────────────┘   │
//!                    │                                     │
//!                    │  ┌─────────────────────────────┐   │
//!                    │  │  PeerAcceptorActor (each)   │   │
//!                    │  │  - Connects to peer         │   │
//!                    │  │  - Sends sync proposals     │   │
//!                    │  │  - Receives accepts         │   │
//!                    │  └─────────────────────────────┘   │
//!                    └─────────────────────────────────────┘
//! ```
//!
//! # Learning Protocol
//!
//! Learners act as "pseudo-proposers" by sending a sync proposal (Prepare)
//! to initiate the learning stream from each acceptor. The acceptor responds
//! with all accepted values from that round onwards.

use std::collections::BTreeMap;

use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, EndpointAddr};
use tokio::sync::{mpsc, watch};
use universal_sync_core::{AcceptorId, Epoch, GroupId, GroupMessage, GroupProposal, PAXOS_ALPN};
use universal_sync_paxos::AcceptorStateStore;
use universal_sync_paxos::proposer::QuorumTracker;

use crate::acceptor::GroupAcceptor;
use crate::connector::ConnectorError;
use crate::state_store::GroupStateStore;

/// Events sent from peer acceptor actors to the learning coordinator
#[derive(Debug)]
enum PeerEvent {
    /// Peer accepted a value
    Accepted {
        /// The acceptor ID that sent this
        acceptor_id: AcceptorId,
        /// The accepted proposal
        proposal: GroupProposal,
        /// The accepted message (boxed to reduce enum size)
        message: Box<GroupMessage>,
    },
    /// Peer connection failed
    Disconnected {
        /// The acceptor ID that disconnected
        acceptor_id: AcceptorId,
        /// Error message
        error: String,
    },
}

/// Learning actor that coordinates quorum tracking across acceptors
///
/// This actor:
/// - Subscribes to local state store for accepted values
/// - Manages connections to peer acceptors
/// - Tracks accepted values from all sources (local + peers)
/// - Notifies epoch watcher when quorum is reached
pub(crate) struct GroupLearningActor<C, CS>
where
    C: mls_rs::external_client::builder::MlsConfig + Clone + 'static,
    CS: mls_rs::CipherSuiteProvider + 'static,
{
    /// This acceptor's ID
    own_id: AcceptorId,
    /// The group ID
    group_id: GroupId,
    /// Iroh endpoint for peer connections
    endpoint: Endpoint,
    /// Current acceptor addresses (excluding self)
    peer_acceptors: BTreeMap<AcceptorId, EndpointAddr>,
    /// Quorum tracker
    quorum_tracker: QuorumTracker<GroupAcceptor<C, CS>>,
    /// Channel for receiving peer events
    peer_rx: mpsc::Receiver<PeerEvent>,
    /// Channel sender for peer actors to send events
    peer_tx: mpsc::Sender<PeerEvent>,
    /// Epoch watcher to notify when quorum reached
    epoch_tx: watch::Sender<Epoch>,
    /// Current peer actor handles (for cleanup)
    peer_handles: BTreeMap<AcceptorId, tokio::task::JoinHandle<()>>,
}

impl<C, CS> GroupLearningActor<C, CS>
where
    C: mls_rs::external_client::builder::MlsConfig + Clone + Send + Sync + 'static,
    CS: mls_rs::CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Create a new learning actor
    ///
    /// # Arguments
    /// * `own_id` - This acceptor's ID
    /// * `group_id` - The group being learned
    /// * `endpoint` - Iroh endpoint for peer connections
    /// * `initial_acceptors` - Initial set of acceptor addresses
    /// * `epoch_tx` - Watch channel to notify epoch changes
    #[must_use]
    pub(crate) fn new(
        own_id: AcceptorId,
        group_id: GroupId,
        endpoint: Endpoint,
        initial_acceptors: impl IntoIterator<Item = (AcceptorId, EndpointAddr)>,
        epoch_tx: watch::Sender<Epoch>,
    ) -> Self {
        let (peer_tx, peer_rx) = mpsc::channel(256);

        // Filter out self from peer acceptors
        let peer_acceptors: BTreeMap<_, _> = initial_acceptors
            .into_iter()
            .filter(|(id, _)| *id != own_id)
            .collect();

        // Total acceptors includes self
        let total_acceptors = peer_acceptors.len() + 1;

        Self {
            own_id,
            group_id,
            endpoint,
            peer_acceptors,
            quorum_tracker: QuorumTracker::new(total_acceptors),
            peer_rx,
            peer_tx,
            epoch_tx,
            peer_handles: BTreeMap::new(),
        }
    }

    /// Run the learning actor
    ///
    /// Subscribes to the local state store and peer acceptors, tracking quorum
    /// and notifying the epoch watcher when values reach consensus.
    ///
    /// # Errors
    ///
    /// Returns an error if the state store subscription fails.
    pub(crate) async fn run(mut self, state: GroupStateStore, initial_round: Epoch) {
        // Subscribe to local state store for accepted values
        let mut local_subscription =
            AcceptorStateStore::<GroupAcceptor<C, CS>>::subscribe_from(&state, initial_round).await;

        // Spawn peer actors for initial acceptors
        let peers: Vec<_> = self
            .peer_acceptors
            .iter()
            .map(|(id, addr)| (*id, addr.clone()))
            .collect();
        for (id, addr) in peers {
            self.spawn_peer_actor(id, addr, initial_round);
        }

        loop {
            tokio::select! {
                // Handle local accepted values
                Some((proposal, message)) = local_subscription.next() => {
                    self.handle_accepted(self.own_id, proposal, message);
                }

                // Handle peer events
                Some(event) = self.peer_rx.recv() => {
                    match event {
                        PeerEvent::Accepted { acceptor_id, proposal, message } => {
                            self.handle_accepted(acceptor_id, proposal, *message);
                        }
                        PeerEvent::Disconnected { acceptor_id, error } => {
                            // Peer actor handles its own reconnection, this is just informational
                            tracing::debug!(?acceptor_id, %error, "peer disconnected (will reconnect)");
                        }
                    }
                }

                else => break,
            }
        }

        // Cleanup: abort peer actors
        for handle in self.peer_handles.values() {
            handle.abort();
        }
        self.peer_handles.clear();
    }

    /// Handle an accepted value (from local or peer)
    fn handle_accepted(
        &mut self,
        _acceptor_id: AcceptorId,
        proposal: GroupProposal,
        message: GroupMessage,
    ) {
        let current_epoch = *self.epoch_tx.borrow();

        // Skip old rounds
        if proposal.epoch < current_epoch {
            return;
        }

        // Track and check for quorum
        if let Some((p, _m)) = self.quorum_tracker.track(proposal, message) {
            // Quorum reached - notify epoch watcher
            let new_epoch = Epoch(p.epoch.0 + 1);
            tracing::info!(epoch = ?p.epoch, "quorum reached");
            let _ = self.epoch_tx.send(new_epoch);
        }
    }

    /// Spawn a peer actor for connecting to a peer acceptor
    fn spawn_peer_actor(&mut self, id: AcceptorId, addr: EndpointAddr, current_round: Epoch) {
        let endpoint = self.endpoint.clone();
        let group_id = self.group_id;
        let tx = self.peer_tx.clone();

        let handle = tokio::spawn(async move {
            run_peer_actor(endpoint, addr, group_id, id, current_round, tx).await;
        });

        self.peer_handles.insert(id, handle);
    }
}

/// Maximum reconnection attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 10;
/// Initial reconnection delay
const INITIAL_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
/// Maximum reconnection delay
const MAX_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(30);

/// Run a peer acceptor actor with reconnection logic
///
/// Connects to a peer acceptor, sends a sync proposal to initiate the
/// learning stream, and forwards received accepts to the learning actor.
/// Automatically reconnects with exponential backoff on disconnection.
async fn run_peer_actor(
    endpoint: Endpoint,
    addr: EndpointAddr,
    group_id: GroupId,
    acceptor_id: AcceptorId,
    initial_round: Epoch,
    tx: mpsc::Sender<PeerEvent>,
) {
    let mut reconnect_attempts = 0u32;
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    let mut current_round = initial_round;

    loop {
        // Try to run a connection
        match run_peer_connection(&endpoint, &addr, group_id, acceptor_id, current_round, &tx)
            .await
        {
            Ok(last_epoch) => {
                // Connection closed gracefully, update round for next connection
                current_round = last_epoch;
                // Reset backoff on successful connection that ran for a while
                reconnect_attempts = 0;
                reconnect_delay = INITIAL_RECONNECT_DELAY;
            }
            Err(e) => {
                reconnect_attempts += 1;

                if reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
                    tracing::warn!(
                        ?acceptor_id,
                        %e,
                        "max reconnect attempts reached, giving up on peer"
                    );
                    let _ = tx
                        .send(PeerEvent::Disconnected {
                            acceptor_id,
                            error: format!("max reconnects exceeded: {e}"),
                        })
                        .await;
                    return;
                }

                tracing::debug!(
                    ?acceptor_id,
                    attempt = reconnect_attempts,
                    delay_ms = reconnect_delay.as_millis(),
                    %e,
                    "reconnecting to peer acceptor"
                );
            }
        }

        // Notify about disconnection (informational)
        let _ = tx
            .send(PeerEvent::Disconnected {
                acceptor_id,
                error: "connection closed, reconnecting".to_string(),
            })
            .await;

        // Wait before reconnecting
        tokio::time::sleep(reconnect_delay).await;

        // Exponential backoff with cap
        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

/// Run a single peer connection session
///
/// Returns the last epoch seen on success (for resuming), or an error on failure.
async fn run_peer_connection(
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    group_id: GroupId,
    acceptor_id: AcceptorId,
    current_round: Epoch,
    tx: &mpsc::Sender<PeerEvent>,
) -> Result<Epoch, ConnectorError> {
    use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
    use universal_sync_core::codec::PostcardCodec;
    use universal_sync_core::{Handshake, HandshakeResponse};

    tracing::debug!(?acceptor_id, "connecting to peer acceptor");

    // Connect to the peer acceptor
    let conn = endpoint
        .connect(addr.clone(), PAXOS_ALPN)
        .await
        .map_err(|e| ConnectorError::Connect(e.to_string()))?;

    // Open a bidirectional stream
    let (send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| ConnectorError::Connect(e.to_string()))?;

    // Handshake
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(16 * 1024 * 1024)
        .new_codec();
    let mut reader = FramedRead::new(recv, codec.clone());
    let mut writer = FramedWrite::new(send, codec);

    // Send Join handshake
    let handshake = Handshake::JoinProposals(group_id);
    let handshake_bytes =
        postcard::to_allocvec(&handshake).map_err(|e| ConnectorError::Codec(e.to_string()))?;
    writer
        .send(handshake_bytes.into())
        .await
        .map_err(ConnectorError::Io)?;

    // Read response
    let response_bytes = reader
        .next()
        .await
        .ok_or_else(|| ConnectorError::Handshake("connection closed".to_string()))?
        .map_err(ConnectorError::Io)?;

    let response: HandshakeResponse = postcard::from_bytes(&response_bytes)
        .map_err(|e| ConnectorError::Codec(format!("invalid response: {e}")))?;

    match response {
        HandshakeResponse::Ok => {}
        other => {
            return Err(ConnectorError::Handshake(format!("{other:?}")));
        }
    }

    // Extract streams and create protocol readers/writers
    let recv = reader.into_inner();
    let send = writer.into_inner();

    let mut reader: FramedRead<
        iroh::endpoint::RecvStream,
        PostcardCodec<crate::connector::ProposalResponse>,
    > = FramedRead::new(recv, PostcardCodec::new());
    let mut writer: FramedWrite<
        iroh::endpoint::SendStream,
        PostcardCodec<crate::connector::ProposalRequest>,
    > = FramedWrite::new(send, PostcardCodec::new());

    // Send a sync Prepare to initiate the learning stream
    // Use attempt=0 to indicate this is a learning request
    let sync_proposal = GroupProposal {
        member_id: universal_sync_core::MemberId(u32::MAX),
        epoch: current_round,
        attempt: universal_sync_core::Attempt::default(),
        message_hash: [0u8; 32], // Will be filled by actual proposer
        signature: vec![],
    };

    writer
        .send(crate::connector::ProposalRequest::Prepare(sync_proposal))
        .await
        .map_err(|e| ConnectorError::Io(std::io::Error::other(e)))?;

    tracing::debug!(?acceptor_id, "learning stream established");

    // Track the last epoch we saw for resumption
    let mut last_epoch = current_round;

    // Read accepted values and forward to learning actor
    while let Some(result) = reader.next().await {
        let response = result.map_err(|e| ConnectorError::Io(std::io::Error::other(e)))?;

        if let Some((proposal, message)) = response.accepted {
            // Update last epoch for resumption
            if proposal.epoch > last_epoch {
                last_epoch = proposal.epoch;
            }

            tx.send(PeerEvent::Accepted {
                acceptor_id,
                proposal,
                message: Box::new(message),
            })
            .await
            .map_err(|_| ConnectorError::Io(std::io::Error::other("channel closed")))?;
        }
    }

    Ok(last_epoch)
}
