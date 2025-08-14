use core::task::{Context, Poll, Waker};

use futures::{FutureExt as _, TryFutureExt as _};
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{handler::InboundUpgradeSend, ConnectionHandlerEvent},
    StreamProtocol,
};

use crate::{
    core::with_edge::behaviour::handler::{
        dropped::DroppedState, receiving::ReceivingState, ConnectionState, FailureReason,
        FromBehaviour, PollResult, StateTrait, TimerFuture, ToBehaviour, LOG_TARGET,
    },
    recv_msg,
};

/// State reached after a successful inbound stream negotiation.
pub struct ReadyToReceiveState {
    /// The inbound stream as returned by libp2p after successful negotiation.
    inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
    /// The timer future that will be polled regularly to close the connection
    /// when idling for too long.
    timeout_timer: TimerFuture,
    behaviour_notified: bool,
    /// Flag set by the behaviour to handle race conditions in which multiple
    /// streams are upgraded at once before noticing that they are too many.
    go_ahead_received: bool,
    waker: Option<Waker>,
}

impl ReadyToReceiveState {
    pub fn new(
        timeout_timer: TimerFuture,
        inbound_stream: <ReadyUpgrade<StreamProtocol> as InboundUpgradeSend>::Output,
        waker: Option<Waker>,
    ) -> Self {
        // We wake here because we want the timeout to be polled in order to be
        // started.
        if let Some(waker) = waker {
            waker.wake();
        }
        Self {
            inbound_stream,
            timeout_timer,
            behaviour_notified: false,
            go_ahead_received: false,
            waker: None,
        }
    }
}

impl From<ReadyToReceiveState> for ConnectionState {
    fn from(value: ReadyToReceiveState) -> Self {
        Self::ReadyToReceive(value)
    }
}

impl StateTrait for ReadyToReceiveState {
    fn on_behaviour_event(mut self, event: FromBehaviour) -> ConnectionState {
        match event {
            FromBehaviour::CloseSubstream => DroppedState::new(None, self.take_waker()).into(),
            FromBehaviour::StartReceiving => {
                self.go_ahead_received = true;
                if let Some(waker) = self.take_waker() {
                    waker.wake();
                }
                self.into()
            }
        }
    }

    // If the timer elapses, moves the state machine to `DroppedState` with a
    // timeout error. Otherwise, it moves it to `ReceivingState` with the `recv_msg`
    // future to fetch the first message from the stream.
    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        let Poll::Pending = self.timeout_timer.poll_unpin(cx) else {
            tracing::debug!(target: LOG_TARGET, "Timeout reached without starting the reception of the message. Closing the connection.");
            return (
                Poll::Pending,
                DroppedState::new(Some(FailureReason::Timeout), Some(cx.waker().clone())).into(),
            );
        };
        if !self.behaviour_notified {
            self.behaviour_notified = true;
            tracing::trace!(target: LOG_TARGET, "Notifying behaviour about ready to receive state.");
            return (
                Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                    ToBehaviour::SubstreamOpened,
                )),
                self.into(),
            );
        }
        // If the behaviour has not signaled that we can go ahead, we simply wait.
        if !self.go_ahead_received {
            self.waker = Some(cx.waker().clone());
            return (Poll::Pending, self.into());
        }
        tracing::trace!(target: LOG_TARGET, "Transitioning from `ReadyToReceive` to `Receiving`.");
        let receiving_state = ReceivingState::new(
            self.timeout_timer,
            Box::pin(recv_msg(self.inbound_stream).map_ok(|(_, message)| message)),
            Some(cx.waker().clone()),
        );
        let poll_result = if self.behaviour_notified {
            Poll::Pending
        } else {
            self.behaviour_notified = true;
            Poll::Ready(ConnectionHandlerEvent::NotifyBehaviour(
                ToBehaviour::SubstreamOpened,
            ))
        };
        (poll_result, receiving_state.into())
    }

    fn take_waker(&mut self) -> Option<Waker> {
        self.waker.take()
    }
}
