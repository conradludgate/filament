//! Coordinator actor.
//!
//! The [`CoordinatorActor`] owns the [`GroupClient`] and a routing table of
//! document actors. It runs a `select!` loop over:
//!
//! 1. Incoming [`CoordinatorRequest`]s from Tauri commands
//! 2. Incoming welcome messages from the network (for auto-joining)
//!
//! When a group is created or joined, the coordinator spawns a
//! [`DocumentActor`](crate::document::DocumentActor) and stores its sender
//! in the routing table.

use std::collections::HashMap;

use mls_rs::client_builder::MlsConfig;
use mls_rs::CipherSuiteProvider;
use tauri::AppHandle;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use universal_sync_core::GroupId;
use universal_sync_proposer::{Group, GroupClient};

use yrs::Transact;

use crate::document::DocumentActor;
use crate::types::{CoordinatorRequest, DocRequest, DocumentInfo};

/// Central coordinator that owns the `GroupClient` and routes requests to
/// per-document actors.
pub struct CoordinatorActor<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// The high-level group client (MLS + networking + CRDT factories).
    group_client: GroupClient<C, CS>,

    /// Welcome message receiver (split out of GroupClient for non-blocking select).
    welcome_rx: mpsc::Receiver<Vec<u8>>,

    /// Routing table: group ID → document actor sender.
    doc_actors: HashMap<GroupId, mpsc::Sender<DocRequest>>,

    /// Incoming requests from Tauri commands.
    request_rx: mpsc::Receiver<CoordinatorRequest>,

    /// Pending reply for a `RecvWelcome` request (at most one at a time).
    pending_welcome_reply: Option<oneshot::Sender<Result<DocumentInfo, String>>>,

    /// Tauri app handle for spawning document actors.
    app_handle: AppHandle,
}

impl<C, CS> CoordinatorActor<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Create a new coordinator actor.
    pub fn new(
        mut group_client: GroupClient<C, CS>,
        request_rx: mpsc::Receiver<CoordinatorRequest>,
        app_handle: AppHandle,
    ) -> Self {
        let welcome_rx = group_client.take_welcome_rx();
        Self {
            group_client,
            welcome_rx,
            doc_actors: HashMap::new(),
            request_rx,
            pending_welcome_reply: None,
            app_handle,
        }
    }

    /// Run the coordinator loop. Returns when the request channel is closed.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                req = self.request_rx.recv() => {
                    match req {
                        Some(req) => self.handle_request(req).await,
                        None => break,
                    }
                }
                Some(welcome_bytes) = self.welcome_rx.recv() => {
                    self.handle_welcome_received(welcome_bytes).await;
                }
            }
        }

        info!("coordinator shutting down");
    }

    /// Dispatch a coordinator request.
    async fn handle_request(&mut self, request: CoordinatorRequest) {
        match request {
            CoordinatorRequest::CreateDocument { reply } => {
                let result = self.create_document().await;
                let _ = reply.send(result);
            }
            CoordinatorRequest::GetKeyPackage { reply } => {
                let _ = reply.send(self.get_key_package());
            }
            CoordinatorRequest::RecvWelcome { reply } => {
                // Store the reply sender; we'll respond when a welcome arrives.
                // If there's already a pending welcome request, drop the old one
                // (the caller's oneshot receiver will see an error).
                self.pending_welcome_reply = Some(reply);
            }
            CoordinatorRequest::JoinDocumentBytes {
                welcome_b58,
                reply,
            } => {
                let result = self.join_document_bytes(&welcome_b58).await;
                let _ = reply.send(result);
            }
            CoordinatorRequest::ForDoc { group_id, request } => {
                self.route_to_doc(group_id, request).await;
            }
        }
    }

    // =========================================================================
    // Coordinator-level operations
    // =========================================================================

    async fn create_document(&mut self) -> Result<DocumentInfo, String> {
        let group = self
            .group_client
            .create_group(&[], "yrs")
            .await
            .map_err(|e| format!("failed to create group: {e:?}"))?;

        self.register_document(group)
    }

    fn get_key_package(&self) -> Result<String, String> {
        let kp = self
            .group_client
            .generate_key_package()
            .map_err(|e| format!("failed to generate key package: {e:?}"))?;
        let bytes = kp
            .to_bytes()
            .map_err(|e| format!("failed to serialize key package: {e:?}"))?;
        Ok(bs58::encode(bytes).into_string())
    }

    async fn join_document_bytes(&mut self, welcome_b58: &str) -> Result<DocumentInfo, String> {
        let welcome_bytes = bs58::decode(welcome_b58)
            .into_vec()
            .map_err(|e| format!("invalid base58: {e}"))?;
        self.join_with_welcome(&welcome_bytes).await
    }

    async fn handle_welcome_received(&mut self, welcome_bytes: Vec<u8>) {
        match self.join_with_welcome(&welcome_bytes).await {
            Ok(doc_info) => {
                // If someone is waiting for a welcome, reply to them.
                if let Some(reply) = self.pending_welcome_reply.take() {
                    let _ = reply.send(Ok(doc_info));
                }
            }
            Err(e) => {
                warn!(?e, "failed to join from welcome");
                if let Some(reply) = self.pending_welcome_reply.take() {
                    let _ = reply.send(Err(e));
                }
            }
        }
    }

    async fn join_with_welcome(&mut self, welcome_bytes: &[u8]) -> Result<DocumentInfo, String> {
        let group = self
            .group_client
            .join_group(welcome_bytes)
            .await
            .map_err(|e| format!("failed to join group: {e:?}"))?;

        self.register_document(group)
    }

    // =========================================================================
    // Document actor lifecycle
    // =========================================================================

    /// Spawn a [`DocumentActor`] for the given group and register it.
    fn register_document(&mut self, group: Group<C, CS>) -> Result<DocumentInfo, String> {
        let group_id = group.group_id();
        let group_id_b58 = bs58::encode(group_id.as_bytes()).into_string();

        // Read initial text (empty for new groups, snapshot for joined groups).
        let text = {
            let yrs = group
                .crdt()
                .as_any()
                .downcast_ref::<universal_sync_testing::YrsCrdt>()
                .ok_or("CRDT is not YrsCrdt")?;
            let text_ref = yrs.doc().get_or_insert_text("doc");
            let txn = yrs.doc().transact();
            yrs::GetString::get_string(&text_ref, &txn)
        };

        let (doc_tx, doc_rx) = mpsc::channel(64);
        let actor = DocumentActor::new(group, group_id, doc_rx, self.app_handle.clone());
        tokio::spawn(actor.run());

        self.doc_actors.insert(group_id, doc_tx);

        info!(%group_id_b58, "document actor spawned");

        Ok(DocumentInfo {
            group_id: group_id_b58,
            text,
            member_count: 1, // will be updated by context queries
        })
    }

    /// Route a request to the correct document actor.
    async fn route_to_doc(&self, group_id: GroupId, request: DocRequest) {
        if let Some(tx) = self.doc_actors.get(&group_id) {
            if tx.send(request).await.is_err() {
                warn!("document actor closed for group");
            }
        }
        // If the group_id isn't found, the oneshot inside `request` is dropped,
        // which causes the caller to see a "RecvError".
    }
}
