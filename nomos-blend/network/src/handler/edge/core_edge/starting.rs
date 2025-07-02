use core::{
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures_timer::Delay;
use libp2p::swarm::handler::FullyNegotiatedInbound;

use crate::handler::edge::core_edge::{
    dropped::DroppedState, ready_to_receive::ReadyToReceiveState, ConnectionEvent, ConnectionState,
    FailureReason, PollResult, StateTrait, LOG_TARGET,
};

pub struct StartingState {
    connection_timeout: Duration,
    waker: Option<Waker>,
}

impl StartingState {
    pub const fn new(connection_timeout: Duration) -> Self {
        Self {
            connection_timeout,
            waker: None,
        }
    }
}

impl From<StartingState> for ConnectionState {
    fn from(value: StartingState) -> Self {
        Self::Starting(value)
    }
}

impl StateTrait for StartingState {
    fn on_connection_event(mut self, event: ConnectionEvent) -> ConnectionState {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol: inbound_stream,
                ..
            }) => {
                tracing::trace!(target: LOG_TARGET, "Transitioning from `Starting` to `ReadyToReceive`.");
                // We awake because the new state has the timeout future which must be polled
                // once.
                if let Some(waker) = self.waker.take() {
                    waker.wake();
                }
                ReadyToReceiveState::new(
                    Box::pin(Delay::new(self.connection_timeout)),
                    inbound_stream,
                )
                .into()
            }
            ConnectionEvent::ListenUpgradeError(error) => {
                tracing::trace!(target: LOG_TARGET, "Inbound upgrade error: {error:?}");
                if let Some(waker) = self.waker.take() {
                    waker.wake();
                }
                DroppedState::new(Some(FailureReason::UpgradeError)).into()
            }
            unprocessed_event => {
                tracing::trace!(target: LOG_TARGET, "Ignoring connection event {unprocessed_event:?}");
                self.into()
            }
        }
    }

    fn poll(mut self, cx: &mut Context<'_>) -> PollResult<ConnectionState> {
        self.waker = Some(cx.waker().clone());
        (Poll::Pending, self.into())
    }
}
