//! Command types for the collaborative editor
//!
//! These types provide the IPC interface between the frontend and backend.

use serde::{Deserialize, Serialize};
use universal_sync_core::GroupId;

/// Information about a document returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    /// The group ID as base58
    pub group_id: String,
    /// Current text content
    pub text: String,
    /// Number of members in the group
    pub member_count: usize,
}

/// A text editing delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeltaCommand {
    /// Insert text at position
    Insert {
        /// UTF-8 position
        position: u32,
        /// Text to insert
        text: String,
    },
    /// Delete text at position
    Delete {
        /// UTF-8 position
        position: u32,
        /// Number of characters to delete
        length: u32,
    },
}

impl From<DeltaCommand> for crate::TextDelta {
    fn from(cmd: DeltaCommand) -> Self {
        match cmd {
            DeltaCommand::Insert { position, text } => crate::TextDelta::Insert { position, text },
            DeltaCommand::Delete { position, length } => {
                crate::TextDelta::Delete { position, length }
            }
        }
    }
}

/// Parse a group ID from base58.
///
/// # Errors
/// Returns an error string if the base58 is invalid or too long.
pub fn parse_group_id(b58: &str) -> Result<GroupId, String> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .map_err(|e| format!("invalid base58: {e}"))?;
    if bytes.len() > 32 {
        return Err("group ID too long (max 32 bytes)".to_string());
    }
    Ok(GroupId::from_slice(&bytes))
}
