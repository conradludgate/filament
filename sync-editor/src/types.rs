//! Request/response types for the editor actor system.
//!
//! These types are intentionally non-generic so they can be held in Tauri's
//! managed state without leaking MLS type parameters.

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use universal_sync_core::GroupId;

// =============================================================================
// Tauri-managed application state
// =============================================================================

/// Application state managed by Tauri.
///
/// Contains only an `mpsc::Sender` — no mutexes, no shared mutable state.
pub struct AppState {
    /// Channel to send requests to the [`CoordinatorActor`](crate::actor::CoordinatorActor).
    pub coordinator_tx: mpsc::Sender<CoordinatorRequest>,
}

// =============================================================================
// Data types exchanged with the frontend
// =============================================================================

/// A text editing operation.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Delta {
    /// Insert text at a position.
    Insert { position: u32, text: String },
    /// Delete `length` characters starting at `position`.
    Delete { position: u32, length: u32 },
    /// Replace `length` characters at `position` with `text`.
    Replace {
        position: u32,
        length: u32,
        text: String,
    },
}

/// Information about an open document, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub group_id: String,
    pub text: String,
    pub member_count: usize,
}

/// Payload emitted to the frontend when a document is updated by a remote peer.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentUpdatedPayload {
    pub group_id: String,
    pub text: String,
}

// =============================================================================
// Coordinator ↔ Tauri command messages
// =============================================================================

/// Request sent from Tauri commands to the [`CoordinatorActor`](crate::actor::CoordinatorActor).
pub enum CoordinatorRequest {
    /// Create a new document (group with Yrs CRDT).
    CreateDocument {
        reply: oneshot::Sender<Result<DocumentInfo, String>>,
    },
    /// Generate a key package for joining a group.
    GetKeyPackage {
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Wait for an incoming welcome message, auto-join, and return the document.
    RecvWelcome {
        reply: oneshot::Sender<Result<DocumentInfo, String>>,
    },
    /// Join a group from raw welcome bytes (base58-encoded).
    JoinDocumentBytes {
        welcome_b58: String,
        reply: oneshot::Sender<Result<DocumentInfo, String>>,
    },
    /// Forward a request to a specific document actor.
    ForDoc {
        group_id: GroupId,
        request: DocRequest,
    },
}

// =============================================================================
// DocumentActor messages
// =============================================================================

/// Request sent from the coordinator (or commands) to a [`DocumentActor`](crate::document::DocumentActor).
pub enum DocRequest {
    /// Apply a text editing delta and broadcast to peers.
    ApplyDelta {
        delta: Delta,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Read the current document text.
    GetText {
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Add a member to the group.
    AddMember {
        key_package_b58: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Add an acceptor to the group.
    AddAcceptor {
        addr_b58: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// List the group's acceptors.
    ListAcceptors {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    /// Shut down the document actor.
    #[allow(dead_code)]
    Shutdown,
}
