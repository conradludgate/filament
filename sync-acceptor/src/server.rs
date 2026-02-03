//! Iroh-based server for Paxos acceptors
//!
//! This module provides the server-side connection handling for acceptors,
//! accepting incoming iroh connections and running the Paxos acceptor protocol.
//!
//! # Example
//!
//! ```ignore
//! use universal_sync_acceptor::{accept_connection, GroupRegistry};
//!
//! let endpoint = iroh::Endpoint::builder()
//!     .alpns(vec![universal_sync_acceptor::PAXOS_ALPN.to_vec()])
//!     .bind()
//!     .await?;
//!
//! let registry = MyGroupRegistry::new();
//!
//! // Accept connections in a loop
//! while let Some(incoming) = endpoint.accept().await {
//!     let registry = registry.clone();
//!     tokio::spawn(async move {
//!         if let Err(e) = accept_connection(incoming, registry).await {
//!             eprintln!("connection error: {e}");
//!         }
//!     });
//! }
//! ```

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Sink, SinkExt, Stream, StreamExt};
use iroh::endpoint::{Incoming, RecvStream, SendStream};
use pin_project_lite::pin_project;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::{debug, warn};
use universal_sync_core::{
    GroupId, GroupMessage, GroupProposal, Handshake, HandshakeResponse, MemberId,
};
use universal_sync_paxos::acceptor::{AcceptorHandler, run_acceptor};
use universal_sync_paxos::{AcceptorMessage, AcceptorRequest, AcceptorStateStore, Learner};

use crate::connector::{ConnectorError, PAXOS_ALPN};

pin_project! {
    /// Server-side acceptor connection over iroh
    ///
    /// This is the inverse of `IrohConnection`:
    /// - Stream yields `AcceptorRequest` (from clients)
    /// - Sink accepts `AcceptorMessage` (to clients)
    pub struct IrohAcceptorConnection<A> {
        #[pin]
        writer: FramedWrite<SendStream, LengthDelimitedCodec>,
        #[pin]
        reader: FramedRead<RecvStream, LengthDelimitedCodec>,
        _marker: PhantomData<fn() -> A>,
    }
}

impl<A> IrohAcceptorConnection<A> {
    /// Create a new acceptor connection from iroh streams
    #[must_use]
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(16 * 1024 * 1024) // 16 MB max message size
            .new_codec();

        Self {
            writer: FramedWrite::new(send, codec.clone()),
            reader: FramedRead::new(recv, codec),
            _marker: PhantomData,
        }
    }
}

impl<A> Stream for IrohAcceptorConnection<A>
where
    A: Learner<Proposal = GroupProposal, Message = GroupMessage>,
    A::Error: From<ConnectorError>,
{
    type Item = Result<AcceptorRequest<A>, A::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.reader.poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => match postcard::from_bytes(&bytes) {
                Ok(msg) => Poll::Ready(Some(Ok(msg))),
                Err(e) => Poll::Ready(Some(Err(ConnectorError::Codec(e.to_string()).into()))),
            },
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(ConnectorError::Io(e).into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<A> Sink<AcceptorMessage<A>> for IrohAcceptorConnection<A>
where
    A: Learner<Proposal = GroupProposal, Message = GroupMessage>,
    A::Error: From<ConnectorError>,
{
    type Error = A::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project()
            .writer
            .poll_ready(cx)
            .map_err(|e| ConnectorError::Io(e).into())
    }

    fn start_send(self: Pin<&mut Self>, item: AcceptorMessage<A>) -> Result<(), Self::Error> {
        let bytes =
            postcard::to_allocvec(&item).map_err(|e| ConnectorError::Codec(e.to_string()))?;

        self.project()
            .writer
            .start_send(bytes.into())
            .map_err(|e| ConnectorError::Io(e).into())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project()
            .writer
            .poll_flush(cx)
            .map_err(|e| ConnectorError::Io(e).into())
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project()
            .writer
            .poll_close(cx)
            .map_err(|e| ConnectorError::Io(e).into())
    }
}

/// Registry for looking up and creating groups
///
/// Implementations should handle:
/// - Looking up existing groups by ID
/// - Creating new groups from `GroupInfo`
/// - Managing per-group state stores
pub trait GroupRegistry {
    /// The acceptor type for groups
    type Acceptor: universal_sync_paxos::Acceptor<Proposal = GroupProposal, Message = GroupMessage>;

    /// The state store type for groups
    type StateStore: AcceptorStateStore<Self::Acceptor>;

    /// Look up an existing group by ID
    ///
    /// Returns the acceptor and state store if the group exists.
    fn get_group(&self, group_id: &GroupId) -> Option<(Self::Acceptor, Self::StateStore)>;

    /// Create a new group from `GroupInfo` bytes
    ///
    /// The bytes are a serialized MLS `MlsMessage` containing `GroupInfo`.
    /// Returns the group ID, acceptor, and state store if successful.
    ///
    /// # Errors
    /// Returns an error if parsing or joining the group fails.
    fn create_group(
        &self,
        group_info: &[u8],
    ) -> Result<(GroupId, Self::Acceptor, Self::StateStore), String>;
}

/// Accept a single incoming Paxos connection and run the acceptor protocol
///
/// This function:
/// 1. Accepts the incoming iroh connection
/// 2. Validates the ALPN protocol
/// 3. Reads the handshake message to determine the group
/// 4. Looks up or creates the group via the registry
/// 5. Runs the Paxos acceptor protocol until the connection closes
///
/// # Arguments
/// * `incoming` - The incoming iroh connection
/// * `registry` - The group registry for looking up/creating groups
///
/// # Returns
/// Returns `Ok(())` when the connection closes normally.
///
/// # Errors
/// Returns an error if the connection fails, handshake is invalid, or the group cannot be found/created.
pub async fn accept_connection<R>(incoming: Incoming, registry: R) -> Result<(), ConnectorError>
where
    R: GroupRegistry,
    <R::Acceptor as Learner>::Error: From<ConnectorError>,
{
    // Accept the connection
    let conn = incoming
        .accept()
        .map_err(|e| ConnectorError::Connect(e.to_string()))?
        .await
        .map_err(|e| ConnectorError::Connect(e.to_string()))?;

    // Check ALPN
    let alpn = conn.alpn();
    if alpn != PAXOS_ALPN {
        warn!(?alpn, "unexpected ALPN, closing connection");
        return Err(ConnectorError::Connect("unexpected ALPN".to_string()));
    }

    // Get the remote node ID
    let remote_id = conn.remote_id();
    debug!(?remote_id, "accepted connection");

    // Accept the bidirectional stream
    let (send, recv) = conn
        .accept_bi()
        .await
        .map_err(|e| ConnectorError::Connect(e.to_string()))?;

    // Create framed reader/writer for handshake
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(16 * 1024 * 1024)
        .new_codec();
    let mut reader = FramedRead::new(recv, codec.clone());
    let mut writer = FramedWrite::new(send, codec);

    // Read the handshake message
    let handshake_bytes = reader
        .next()
        .await
        .ok_or_else(|| ConnectorError::Connect("connection closed before handshake".to_string()))?
        .map_err(ConnectorError::Io)?;

    let handshake: Handshake = postcard::from_bytes(&handshake_bytes)
        .map_err(|e| ConnectorError::Codec(format!("invalid handshake: {e}")))?;

    // Process handshake and get group
    let (group_id, acceptor, state) = match handshake {
        Handshake::Join(group_id) => {
            if let Some((acceptor, state)) = registry.get_group(&group_id) {
                (group_id, acceptor, state)
            } else {
                // Send error response
                let response = HandshakeResponse::GroupNotFound;
                let response_bytes = postcard::to_allocvec(&response)
                    .map_err(|e| ConnectorError::Codec(e.to_string()))?;
                writer
                    .send(response_bytes.into())
                    .await
                    .map_err(ConnectorError::Io)?;
                return Err(ConnectorError::Connect("group not found".to_string()));
            }
        }
        Handshake::Create(group_info) => {
            match registry.create_group(&group_info) {
                Ok((group_id, acceptor, state)) => (group_id, acceptor, state),
                Err(e) => {
                    // Send error response
                    let response = HandshakeResponse::InvalidGroupInfo(e.clone());
                    let response_bytes = postcard::to_allocvec(&response)
                        .map_err(|e| ConnectorError::Codec(e.to_string()))?;
                    writer
                        .send(response_bytes.into())
                        .await
                        .map_err(ConnectorError::Io)?;
                    return Err(ConnectorError::Connect(format!(
                        "failed to create group: {e}"
                    )));
                }
            }
        }
    };

    debug!(?group_id, "handshake complete");

    // Send success response
    let response = HandshakeResponse::Ok;
    let response_bytes =
        postcard::to_allocvec(&response).map_err(|e| ConnectorError::Codec(e.to_string()))?;
    writer
        .send(response_bytes.into())
        .await
        .map_err(ConnectorError::Io)?;

    // Extract inner streams from framed wrappers
    let recv = reader.into_inner();
    let send = writer.into_inner();

    // Create the connection wrapper for Paxos protocol
    let connection = IrohAcceptorConnection::<R::Acceptor>::new(send, recv);

    // Create the handler with shared state
    let handler = AcceptorHandler::new(acceptor, state);

    // Run the acceptor protocol
    let proposer_id = MemberId(0);

    run_acceptor(handler, connection, proposer_id)
        .await
        .map_err(|e| ConnectorError::Connect(e.to_string()))?;

    debug!(?remote_id, "connection closed");
    Ok(())
}
