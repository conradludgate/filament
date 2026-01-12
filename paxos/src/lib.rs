use futures::{Sink, Stream, TryStream};
use zerocopy::{FromBytes, Immutable, IntoBytes, TryFromBytes};

pub trait Learner {
    type EpochIdentifier: IntoBytes + FromBytes + Immutable + Ord;
    type Message: IntoBytes + FromBytes + Immutable;
    type Error: core::error::Error;

    /// The current state epoch.
    fn epoch(&self) -> Self::EpochIdentifier;

    /// Propose a new message be added to the current state
    async fn propose(&mut self, message: Self::Message) -> Result<(), Self::Error>;

    /// Commit the most recently proposed message
    async fn commit(&mut self) -> Result<(), Self::Error>;

    /// Rollback the latest proposed message.
    async fn rollback(&mut self) -> Result<(), Self::Error>;
}

/// A proposer
pub trait Proposer {
    type Error: core::error::Error;
    type State: Learner<Error = Self::Error>;
    type Acceptor<'a>: Sink<Prepare<Self::State>, Error = Self::Error>
        + Sink<Accept<Self::State>, Error = Self::Error>
        + TryStream<Ok = PromiseOrNack<Self::State>, Error = Self::Error>
    where
        Self: 'a;

    /// Return the list of acceptors this proposer will use to form consensus.
    fn acceptors(&mut self) -> impl IntoIterator<Item = Self::Acceptor<'_>>;

    /// Resync the current state with the peers.
    ///
    /// This proposer has fallen behind on the current protocol state and cannot
    /// propose again until it catches back up.
    async fn sync(
        &mut self,
        state: &mut Self::State,
        epoch: <Self::State as Learner>::EpochIdentifier,
    ) -> Result<(), Self::Error>;

    /// Wait for the next proposal to make
    ///
    /// Must be cancel-safe
    async fn next_proposal(
        state: &mut Self::State,
    ) -> Result<<Self::State as Learner>::Message, Self::Error>;
}

pub struct Prepare<S: Learner>(S::EpochIdentifier);
pub struct Accept<S: Learner>(S::EpochIdentifier, S::Message);
pub enum PromiseOrNack<S: Learner> {
    Promise(S::EpochIdentifier),
    Nack(S::EpochIdentifier),
}

async fn run_proposer<P>(mut p: P, mut state: P::State) -> Result<(), P::Error>
where
    P: Proposer,
{
    loop {
        let mut acceptors: Vec<_> = p.acceptors().into_iter().collect();

        tokio::select! {
            proposal = P::next_proposal(&mut state) => {
                proposal?;
                propose(&mut acceptors, proposal).await;
                state.rollback().await?;
            }
        }
    }
}

async fn learn<A, S, E>(acceptors: &mut [A])
where
    E: core::error::Error,
    S: Learner<Error = E>,
    A: Sink<Prepare<S>, Error = E>
        + Sink<Accept<S>, Error = E>
        + TryStream<Ok = PromiseOrNack<S>, Error = E>,
{
    
}

// /// A connection to an Acceptor
// pub trait AcceptorLink {
//     type Error: core::error::Error;

//     /// Send a paxos message to the acceptor
//     ///
//     /// Must be cancel-safe
//     async fn send(&mut self, msg: &[u8]) -> Result<(), Self::Error>;

//     /// Receive a paxos message from the acceptor
//     ///
//     /// Must be cancel-safe
//     async fn recv(&mut self) -> Result<Vec<u8>, Self::Error>;
// }

// struct P {
//     acceptors: Vec<A>,
// }

// impl Proposer for P {
//     type EpochIdentifier = ();
//     type Acceptor<'a>
//         = &'a mut A
//     where
//         Self: 'a;

//     fn epoch(&self) -> Self::EpochIdentifier {}

//     fn acceptors(&mut self) -> impl IntoIterator<Item = Self::Acceptor<'_>> {
//         &mut self.acceptors
//     }
// }

// struct A {}

// impl AcceptorLink for &mut A {}
