//! Lazy connection wrapper
//!
//! Provides [`LazyConnection`], a connection wrapper that establishes
//! the actual connection on first use.

use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures::future::{Fuse, FusedFuture};
use futures::{FutureExt, Sink, Stream};
use pin_project_lite::pin_project;
use tokio::task::coop;
use tracing::{trace, warn};

use crate::messages::{AcceptorMessage, AcceptorRequest};
use crate::traits::{Connector, Learner};

pin_project! {
    /// A lazy connection that establishes the actual connection on first use.
    ///
    /// This implements `Stream + Sink` and will connect during the first
    /// `poll_ready()` or `poll_next()` call.
    ///
    /// The connector is responsible for implementing retry logic with backoff.
    /// Tracks consecutive failures so callers can check connection health.
    pub struct LazyConnection<L: Learner, C: Connector<L>> {
        acceptor_id: L::AcceptorId,
        connector: C,
        #[pin]
        connecting: Fuse<C::ConnectFuture>,
        #[pin]
        connection: Option<C::Connection>,
        // Count of consecutive connection failures
        consecutive_failures: u32,
    }
}

impl<L: Learner, C: Connector<L>> LazyConnection<L, C> {
    /// Create a new lazy connection to the given acceptor.
    pub fn new(acceptor_id: L::AcceptorId, connector: C) -> Self {
        Self {
            acceptor_id,
            connector,
            connecting: Fuse::terminated(),
            connection: None,
            consecutive_failures: 0,
        }
    }
}

/// Macro to poll the connection, establishing it if necessary.
/// The closure receives `Pin<&mut C::Connection>` when ready.
macro_rules! poll_connect {
    ($self:expr, $cx:expr, |$conn:ident| $body:expr) => {{
        let mut this = $self.project();
        loop {
            if let Some($conn) = this.connection.as_mut().as_pin_mut() {
                break $body;
            }

            if !this.connecting.is_terminated() {
                let coop = ready!(coop::poll_proceed($cx));
                let res = ready!(this.connecting.as_mut().poll($cx));
                coop.made_progress();

                if let Ok(conn) = res {
                    trace!("connection established");
                    this.connection.set(Some(conn));
                    *this.consecutive_failures = 0;
                    continue;
                }
                *this.consecutive_failures += 1;
                warn!(
                    failures = *this.consecutive_failures,
                    "connection attempt failed"
                );
                // Try again - connector handles backoff
            }

            // Start a new connection attempt
            // The connector is responsible for backoff and returning an error when giving up
            trace!("starting connection attempt");
            let fut = this.connector.connect(&*this.acceptor_id);
            this.connecting.set(fut.fuse());
        }
    }};
}

impl<L, C> Stream for LazyConnection<L, C>
where
    L: Learner,
    C: Connector<L>,
{
    type Item = Result<AcceptorMessage<L>, L::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        poll_connect!(self, cx, |conn| conn.poll_next(cx))
    }
}

impl<L, C> Sink<AcceptorRequest<L>> for LazyConnection<L, C>
where
    L: Learner,
    C: Connector<L>,
{
    type Error = L::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        poll_connect!(self, cx, |conn| conn.poll_ready(cx))
    }

    fn start_send(self: Pin<&mut Self>, item: AcceptorRequest<L>) -> Result<(), Self::Error> {
        self.project()
            .connection
            .as_pin_mut()
            .expect("start_send called before poll_ready returned Ready")
            .start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.project().connection.as_pin_mut() {
            Some(conn) => conn.poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.project().connection.as_pin_mut() {
            Some(conn) => conn.poll_close(cx),
            None => Poll::Ready(Ok(())),
        }
    }
}
