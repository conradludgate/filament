//! Per-document actor.
//!
//! Each open document is managed by a [`DocumentActor`] that owns a [`Group`]
//! and runs a `select!` loop over:
//!
//! 1. Incoming [`DocRequest`]s from the coordinator / Tauri commands
//! 2. Remote CRDT updates from peers (via [`Group::wait_for_update`])
//!
//! No mutexes — the actor is the sole owner of the `Group`.

use mls_rs::client_builder::MlsConfig;
use mls_rs::{CipherSuiteProvider, MlsMessage};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tracing::warn;
use universal_sync_core::GroupId;
use universal_sync_proposer::Group;
use universal_sync_testing::YrsCrdt;
use yrs::{GetString, Text, Transact};

use crate::types::{Delta, DocRequest, DocumentUpdatedPayload};

/// Actor that manages a single open document / MLS group.
pub struct DocumentActor<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Send + Sync + 'static,
{
    group: Group<C, CS>,
    group_id_b58: String,
    request_rx: mpsc::Receiver<DocRequest>,
    app_handle: AppHandle,
}

impl<C, CS> DocumentActor<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Create a new document actor.
    pub fn new(
        group: Group<C, CS>,
        group_id: GroupId,
        request_rx: mpsc::Receiver<DocRequest>,
        app_handle: AppHandle,
    ) -> Self {
        let group_id_b58 = bs58::encode(group_id.as_bytes()).into_string();
        Self {
            group,
            group_id_b58,
            request_rx,
            app_handle,
        }
    }

    /// Run the actor loop. Returns when the actor is shut down.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                req = self.request_rx.recv() => {
                    match req {
                        Some(req) => {
                            if self.handle_request(req).await {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                update = self.group.wait_for_update() => {
                    match update {
                        Some(()) => self.emit_text_update(),
                        None => break, // group shut down
                    }
                }
            }
        }

        self.group.shutdown().await;
    }

    /// Handle a single request. Returns `true` if shutdown was requested.
    async fn handle_request(&mut self, request: DocRequest) -> bool {
        match request {
            DocRequest::ApplyDelta { delta, reply } => {
                let result = self.apply_delta(delta).await;
                let _ = reply.send(result);
            }
            DocRequest::GetText { reply } => {
                let _ = reply.send(self.get_text());
            }
            DocRequest::AddMember {
                key_package_b58,
                reply,
            } => {
                let result = self.add_member(&key_package_b58).await;
                let _ = reply.send(result);
            }
            DocRequest::AddAcceptor { addr_b58, reply } => {
                let result = self.add_acceptor(&addr_b58).await;
                let _ = reply.send(result);
            }
            DocRequest::ListAcceptors { reply } => {
                let result = self.list_acceptors().await;
                let _ = reply.send(result);
            }
            DocRequest::Shutdown => return true,
        }
        false
    }

    // =========================================================================
    // CRDT helpers
    // =========================================================================

    fn yrs_crdt(&self) -> Result<&YrsCrdt, String> {
        self.group
            .crdt()
            .as_any()
            .downcast_ref::<YrsCrdt>()
            .ok_or_else(|| "CRDT is not YrsCrdt".to_string())
    }

    fn yrs_crdt_mut(&mut self) -> Result<&mut YrsCrdt, String> {
        self.group
            .crdt_mut()
            .as_any_mut()
            .downcast_mut::<YrsCrdt>()
            .ok_or_else(|| "CRDT is not YrsCrdt".to_string())
    }

    // =========================================================================
    // Document operations
    // =========================================================================

    fn get_text(&self) -> Result<String, String> {
        let yrs = self.yrs_crdt()?;
        let text_ref = yrs.doc().get_or_insert_text("doc");
        let txn = yrs.doc().transact();
        Ok(text_ref.get_string(&txn))
    }

    async fn apply_delta(&mut self, delta: Delta) -> Result<(), String> {
        {
            let yrs = self.yrs_crdt_mut()?;
            let text_ref = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            match delta {
                Delta::Insert { position, text } => {
                    text_ref.insert(&mut txn, position, &text);
                }
                Delta::Delete { position, length } => {
                    text_ref.remove_range(&mut txn, position, length);
                }
                Delta::Replace {
                    position,
                    length,
                    text,
                } => {
                    text_ref.remove_range(&mut txn, position, length);
                    text_ref.insert(&mut txn, position, &text);
                }
            }
        }
        self.group
            .send_update()
            .await
            .map_err(|e| format!("failed to send update: {e:?}"))
    }

    async fn add_member(&mut self, key_package_b58: &str) -> Result<(), String> {
        let kp_bytes = bs58::decode(key_package_b58)
            .into_vec()
            .map_err(|e| format!("invalid base58: {e}"))?;
        let key_package = MlsMessage::from_bytes(&kp_bytes)
            .map_err(|e| format!("invalid key package: {e:?}"))?;
        self.group
            .add_member(key_package)
            .await
            .map_err(|e| format!("failed to add member: {e:?}"))
    }

    async fn add_acceptor(&mut self, addr_b58: &str) -> Result<(), String> {
        let bytes = bs58::decode(addr_b58)
            .into_vec()
            .map_err(|e| format!("invalid base58: {e}"))?;
        let addr: iroh::EndpointAddr =
            postcard::from_bytes(&bytes).map_err(|e| format!("invalid address: {e}"))?;
        self.group
            .add_acceptor(addr)
            .await
            .map_err(|e| format!("failed to add acceptor: {e:?}"))
    }

    async fn list_acceptors(&mut self) -> Result<Vec<String>, String> {
        let ctx = self
            .group
            .context()
            .await
            .map_err(|e| format!("failed to get context: {e:?}"))?;
        Ok(ctx
            .acceptors
            .iter()
            .map(|a| bs58::encode(a.as_bytes()).into_string())
            .collect())
    }

    fn emit_text_update(&self) {
        if let Ok(text) = self.get_text() {
            let payload = DocumentUpdatedPayload {
                group_id: self.group_id_b58.clone(),
                text,
            };
            if let Err(e) = self.app_handle.emit("document-updated", &payload) {
                warn!(?e, "failed to emit document-updated event");
            }
        }
    }
}
