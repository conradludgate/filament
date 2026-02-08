//! Error types and structured `error_stack` context types.

use std::fmt;

use crate::{AcceptorId, Epoch, GroupId, MemberId};

/// Network connection error.
#[derive(Debug, Default)]
pub struct ConnectorError;

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("connection failed")
    }
}

impl std::error::Error for ConnectorError {}

/// Error context: group.
#[derive(Debug, Clone)]
pub struct GroupContext {
    pub group_id: GroupId,
}

impl GroupContext {
    #[must_use]
    pub fn new(group_id: GroupId) -> Self {
        Self { group_id }
    }
}

impl fmt::Display for GroupContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "group: {}",
            bs58::encode(&self.group_id.0[..8]).into_string()
        )
    }
}

/// Error context: acceptor.
#[derive(Debug, Clone)]
pub struct AcceptorContext {
    pub acceptor_id: AcceptorId,
}

impl AcceptorContext {
    #[must_use]
    pub fn new(acceptor_id: AcceptorId) -> Self {
        Self { acceptor_id }
    }
}

impl fmt::Display for AcceptorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "acceptor: {}",
            bs58::encode(&self.acceptor_id.0[..8]).into_string()
        )
    }
}

/// Error context: epoch.
#[derive(Debug, Clone, Copy)]
pub struct EpochContext {
    pub epoch: Epoch,
}

impl EpochContext {
    #[must_use]
    pub fn new(epoch: Epoch) -> Self {
        Self { epoch }
    }
}

impl fmt::Display for EpochContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "epoch: {}", self.epoch.0)
    }
}

/// Error context: member.
#[derive(Debug, Clone, Copy)]
pub struct MemberContext {
    pub member_id: MemberId,
}

impl MemberContext {
    #[must_use]
    pub fn new(member_id: MemberId) -> Self {
        Self { member_id }
    }
}

impl fmt::Display for MemberContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "member index: {}", self.member_id.0)
    }
}

/// Error context: what operation was in progress.
#[derive(Debug, Clone)]
pub struct OperationContext {
    pub operation: &'static str,
}

impl OperationContext {
    #[must_use]
    pub fn new(operation: &'static str) -> Self {
        Self { operation }
    }
}

impl fmt::Display for OperationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "while {}", self.operation)
    }
}

impl OperationContext {
    pub const CREATING_GROUP: Self = Self {
        operation: "creating group",
    };
    pub const JOINING_GROUP: Self = Self {
        operation: "joining group",
    };
    pub const ADDING_MEMBER: Self = Self {
        operation: "adding member",
    };
    pub const REMOVING_MEMBER: Self = Self {
        operation: "removing member",
    };
    pub const ADDING_ACCEPTOR: Self = Self {
        operation: "adding acceptor",
    };
    pub const REMOVING_ACCEPTOR: Self = Self {
        operation: "removing acceptor",
    };
    pub const PROPOSING: Self = Self {
        operation: "proposing via Paxos",
    };
    pub const CONNECTING: Self = Self {
        operation: "connecting to acceptor",
    };
    pub const VALIDATING_PROPOSAL: Self = Self {
        operation: "validating proposal",
    };
    pub const APPLYING_VALUE: Self = Self {
        operation: "applying learned value",
    };
    pub const SENDING_WELCOME: Self = Self {
        operation: "sending welcome message",
    };
    pub const PROCESSING_MLS: Self = Self {
        operation: "processing MLS message",
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_error_display() {
        assert_eq!(ConnectorError.to_string(), "connection failed");
        // Also covers std::error::Error impl
        let _: &dyn std::error::Error = &ConnectorError;
    }

    #[test]
    fn group_context_display() {
        let ctx = GroupContext::new(GroupId::new([0xAB; 32]));
        let s = ctx.to_string();
        assert!(s.starts_with("group: "));
    }

    #[test]
    fn acceptor_context_display() {
        let ctx = AcceptorContext::new(AcceptorId([0xCD; 32]));
        let s = ctx.to_string();
        assert!(s.starts_with("acceptor: "));
    }

    #[test]
    fn epoch_context_display() {
        let ctx = EpochContext::new(Epoch(42));
        assert_eq!(ctx.to_string(), "epoch: 42");
    }

    #[test]
    fn member_context_display() {
        let ctx = MemberContext::new(MemberId(7));
        assert_eq!(ctx.to_string(), "member index: 7");
    }

    #[test]
    fn operation_context_display() {
        let ctx = OperationContext::new("testing");
        assert_eq!(ctx.to_string(), "while testing");
        assert_eq!(
            OperationContext::CREATING_GROUP.to_string(),
            "while creating group"
        );
    }
}
