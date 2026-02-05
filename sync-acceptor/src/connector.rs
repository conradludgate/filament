//! Connector types and constants

// =============================================================================
// Proposal stream types (for push-based proposer/learning)
// =============================================================================

use universal_sync_core::{GroupMessage, GroupProposal};

/// Wire format for proposal requests
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ProposalRequest {
    /// Phase 1: Prepare
    Prepare(GroupProposal),
    /// Phase 2: Accept
    Accept(GroupProposal, GroupMessage),
}

/// Wire format for proposal responses
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProposalResponse {
    /// Highest promised proposal
    pub(crate) promised: GroupProposal,
    /// Highest accepted (proposal, message) pair
    pub(crate) accepted: Option<(GroupProposal, GroupMessage)>,
}
