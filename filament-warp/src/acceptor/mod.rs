//! Acceptor runtime — handles protocol messages and state persistence.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{Learner, Proposal};

mod handler;
mod runner;

pub use handler::AcceptorHandler;
pub use runner::run_acceptor_with_epoch_waiter;

/// Shared state for an acceptor, allowing multiple connections to coordinate.
///
/// All connections to the same acceptor must share this state.
///
/// Implementations MUST persist state before returning success from
/// `promise()`/`accept()` (use fsync) and reload on restart for crash recovery.
/// Both `promise()` and `accept()` must be atomic per-round.
#[expect(async_fn_in_trait)]
pub trait AcceptorStateStore<L: Learner>: Send + Sync {
    type Subscription: futures::Stream<Item = (L::Proposal, L::Message)> + Send;

    async fn get(&self, round: <L::Proposal as Proposal>::RoundId) -> RoundState<L>;

    /// MUST reject if a higher proposal was already promised or accepted for this round.
    async fn promise(&self, proposal: &L::Proposal) -> Result<(), RoundState<L>>;

    /// MUST reject if a higher proposal was already promised or accepted for this round.
    /// On success, broadcasts to all subscribed learners.
    async fn accept(
        &self,
        proposal: &L::Proposal,
        message: &L::Message,
    ) -> Result<(), RoundState<L>>;

    /// Returns historical values (rounds >= `from_round`) then live broadcasts.
    async fn subscribe_from(
        &self,
        from_round: <L::Proposal as Proposal>::RoundId,
    ) -> Self::Subscription;

    async fn highest_accepted_round(&self) -> Option<<L::Proposal as Proposal>::RoundId>;

    /// Returns historical values only (no live subscription).
    async fn get_accepted_from(
        &self,
        from_round: <L::Proposal as Proposal>::RoundId,
    ) -> Vec<(L::Proposal, L::Message)>;
}

pub struct RoundState<L: Learner> {
    pub promised: Option<L::Proposal>,
    pub accepted: Option<(L::Proposal, L::Message)>,
}

impl<L: Learner> Clone for RoundState<L> {
    fn clone(&self) -> Self {
        Self {
            promised: self.promised.clone(),
            accepted: self.accepted.clone(),
        }
    }
}

impl<L: Learner> Default for RoundState<L> {
    fn default() -> Self {
        Self {
            promised: None,
            accepted: None,
        }
    }
}

/// Messages from proposer/learner to acceptor.
///
/// Learners can send a "dummy" prepare with default `round`/`attempt`/`node_id`
/// to initiate sync without proposing.
#[derive(Debug)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(bound(
        serialize = "L::Proposal: Serialize, L::Message: Serialize",
        deserialize = "L::Proposal: Deserialize<'de>, L::Message: Deserialize<'de>"
    ))
)]
pub enum AcceptorRequest<L: Learner> {
    Prepare(L::Proposal),
    Accept(L::Proposal, L::Message),
}

impl<L: Learner> Clone for AcceptorRequest<L> {
    fn clone(&self) -> Self {
        match self {
            Self::Prepare(p) => Self::Prepare(p.clone()),
            Self::Accept(p, m) => Self::Accept(p.clone(), m.clone()),
        }
    }
}

/// Messages from acceptor to proposer/learner.
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(bound(
        serialize = "L::Proposal: Serialize, L::Message: Serialize",
        deserialize = "L::Proposal: Deserialize<'de>, L::Message: Deserialize<'de>"
    ))
)]
pub struct AcceptorMessage<L: Learner> {
    pub promised: L::Proposal,
    pub accepted: Option<(L::Proposal, L::Message)>,
}

impl<L: Learner> fmt::Debug for AcceptorMessage<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let AcceptorMessage { promised, accepted } = self;
        f.debug_struct("AcceptorMessage")
            .field("promised", promised)
            .field("accepted", accepted)
            .finish()
    }
}

impl<L: Learner> Clone for AcceptorMessage<L> {
    fn clone(&self) -> Self {
        Self {
            promised: self.promised.clone(),
            accepted: self.accepted.clone(),
        }
    }
}

impl<L: Learner> AcceptorMessage<L> {
    /// # Panics
    /// Panics if `promised` is None, which should never happen for valid responses.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn from_round_state(state: RoundState<L>) -> Self {
        Self {
            promised: state
                .promised
                .expect("response must have a promised proposal"),
            accepted: state.accepted,
        }
    }
}

#[cfg(test)]
mod tests {
    use error_stack::Report;

    use super::*;
    use crate::{Proposal, Validated, ValidationError};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TP {
        node: u32,
        round: u64,
        attempt: u32,
    }

    impl Proposal for TP {
        type NodeId = u32;
        type RoundId = u64;
        type AttemptId = u32;
        fn node_id(&self) -> u32 {
            self.node
        }
        fn round(&self) -> u64 {
            self.round
        }
        fn attempt(&self) -> u32 {
            self.attempt
        }
        fn next_attempt(a: u32) -> u32 {
            a + 1
        }
    }

    struct TL;
    impl Learner for TL {
        type Proposal = TP;
        type Message = String;
        type Error = std::io::Error;
        type AcceptorId = u32;
        fn node_id(&self) -> u32 {
            0
        }
        fn current_round(&self) -> u64 {
            0
        }
        fn acceptors(&self) -> impl IntoIterator<Item = u32, IntoIter: ExactSizeIterator> {
            vec![]
        }
        fn propose(&self, _a: u32) -> TP {
            TP {
                node: 0,
                round: 0,
                attempt: 0,
            }
        }
        fn validate(&self, _: &TP) -> Result<Validated, Report<ValidationError>> {
            Ok(Validated::assert_valid())
        }
        async fn apply(&mut self, _: TP, _: String) -> Result<(), std::io::Error> {
            Ok(())
        }
    }

    #[test]
    fn round_state_default() {
        let rs = RoundState::<TL>::default();
        assert!(rs.promised.is_none());
        assert!(rs.accepted.is_none());
    }

    #[test]
    fn round_state_clone() {
        let rs = RoundState::<TL> {
            promised: Some(TP {
                node: 1,
                round: 2,
                attempt: 3,
            }),
            accepted: Some((
                TP {
                    node: 1,
                    round: 2,
                    attempt: 3,
                },
                "hello".to_string(),
            )),
        };
        let cloned = rs.clone();
        assert_eq!(cloned.promised, rs.promised);
        assert_eq!(cloned.accepted, rs.accepted);
    }

    #[test]
    fn acceptor_request_clone() {
        let req: AcceptorRequest<TL> = AcceptorRequest::Prepare(TP {
            node: 1,
            round: 0,
            attempt: 0,
        });
        let cloned = req.clone();
        assert!(matches!(cloned, AcceptorRequest::Prepare(_)));

        let req2: AcceptorRequest<TL> = AcceptorRequest::Accept(
            TP {
                node: 1,
                round: 0,
                attempt: 0,
            },
            "msg".to_string(),
        );
        let cloned2 = req2.clone();
        assert!(matches!(cloned2, AcceptorRequest::Accept(_, _)));
    }

    #[test]
    fn acceptor_message_debug_and_clone() {
        let msg: AcceptorMessage<TL> = AcceptorMessage {
            promised: TP {
                node: 1,
                round: 0,
                attempt: 0,
            },
            accepted: None,
        };
        let debug = format!("{msg:?}");
        assert!(debug.contains("AcceptorMessage"));

        let cloned = msg.clone();
        assert_eq!(cloned.promised, msg.promised);
    }

    #[test]
    fn acceptor_message_from_round_state() {
        let rs = RoundState::<TL> {
            promised: Some(TP {
                node: 1,
                round: 2,
                attempt: 3,
            }),
            accepted: None,
        };
        let msg = AcceptorMessage::from_round_state(rs);
        assert_eq!(msg.promised.node, 1);
        assert!(msg.accepted.is_none());
    }

    #[test]
    #[should_panic(expected = "response must have a promised proposal")]
    fn acceptor_message_from_round_state_panics_without_promise() {
        let rs = RoundState::<TL> {
            promised: None,
            accepted: None,
        };
        let _ = AcceptorMessage::from_round_state(rs);
    }
}
