// ## m4-t01c-core
//! Refresh tickets (HV-INTRO-01; bodies Task 15): every request to the engine answers a
//! [`RefreshTicket`], a future of the catalog generation that contains its result. Dropping a
//! ticket never cancels shared work (single-flight).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot;

use crate::introspector::IntroError;

/// A future of the generation that contains a request's result.
#[derive(Debug)]
pub struct RefreshTicket {
    state: State,
}

#[derive(Debug)]
enum State {
    /// Answered when made (a stub, a refusal); taken by the first poll.
    Ready(Option<Result<u64, IntroError>>),
    /// Answered by the engine.
    Waiting(oneshot::Receiver<Result<u64, IntroError>>),
}

/// The engine's end of a [`RefreshTicket`]: dropping it unanswered resolves the ticket to
/// [`IntroError::Cancelled`].
#[derive(Debug)]
pub struct TicketSender(oneshot::Sender<Result<u64, IntroError>>);

impl TicketSender {
    /// Answers the ticket; a ticket nobody holds any more is ignored.
    pub fn send(self, result: Result<u64, IntroError>) {
        let _ = self.0.send(result);
    }
}

impl RefreshTicket {
    /// A ticket that is already answered.
    pub fn ready(result: Result<u64, IntroError>) -> Self {
        RefreshTicket {
            state: State::Ready(Some(result)),
        }
    }

    /// A ticket answered by [`IntroError::NotImplemented`] (the M4 Task 1c stubs).
    pub fn not_implemented() -> Self {
        Self::ready(Err(IntroError::NotImplemented))
    }

    /// A ticket and the sender that answers it.
    pub fn pending() -> (Self, TicketSender) {
        let (tx, rx) = oneshot::channel();
        (
            RefreshTicket {
                state: State::Waiting(rx),
            },
            TicketSender(tx),
        )
    }
}

impl Future for RefreshTicket {
    type Output = Result<u64, IntroError>;

    #[allow(
        clippy::result_large_err,
        reason = "IntroError carries the server's message by value (MetaError::Server); one per request"
    )]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.state {
            State::Ready(result) => {
                Poll::Ready(result.take().unwrap_or(Err(IntroError::Cancelled)))
            }
            State::Waiting(rx) => Pin::new(rx)
                .poll(cx)
                .map(|r| r.unwrap_or(Err(IntroError::Cancelled))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tickets_resolve_to_what_they_were_given_or_cancelled() {
        assert_eq!(RefreshTicket::ready(Ok(7)).await, Ok(7));
        assert_eq!(
            RefreshTicket::not_implemented().await,
            Err(IntroError::NotImplemented)
        );
        let (t, tx) = RefreshTicket::pending();
        tx.send(Ok(3));
        assert_eq!(t.await, Ok(3));
        let (t, tx) = RefreshTicket::pending();
        drop(tx);
        assert_eq!(t.await, Err(IntroError::Cancelled));
    }
}
