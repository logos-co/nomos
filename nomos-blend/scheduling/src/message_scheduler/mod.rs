use std::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Stream, StreamExt as _};
use tokio::time;
use tokio_stream::wrappers::IntervalStream;

use crate::{
    cover_traffic_2::SessionCoverTraffic,
    message::OutboundMessage,
    message_scheduler::{round_info::RoundInfo, session_info::SessionInfo},
    release_delayer::SessionReleaseDelayer,
};

pub mod round_info;
pub mod session_info;

pub struct UninitializedMessageScheduler<SessionClock, Rng> {
    rng: Rng,
    session_clock: SessionClock,
    settings: CreationOptions,
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng> {
    pub const fn new(session_clock: SessionClock, settings: CreationOptions, rng: Rng) -> Self {
        Self {
            rng,
            session_clock,
            settings,
        }
    }
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng,
{
    pub async fn wait_next_session_start(self) -> MessageScheduler<SessionClock, Rng> {
        let Self {
            mut rng,
            mut session_clock,
            settings,
        } = self;
        // We wait until the provided session stream returns its first usable value,
        // which we use to initialize the scheduler.
        let first_session_info = async {
            loop {
                if let Some(session_info) = session_clock.next().await {
                    break session_info;
                }
            }
        }
        .await;

        MessageScheduler {
            cover_traffic: settings.cover_traffic(first_session_info.core_quota, &mut rng),
            release_delayer: settings.release_delayer(rng),
            round_clock: settings.round_clock(),
            session_clock,
            settings,
        }
    }
}

pub struct MessageScheduler<SessionClock, Rng> {
    cover_traffic: SessionCoverTraffic,
    release_delayer: SessionReleaseDelayer<Rng>,
    round_clock: Box<dyn Stream<Item = ()> + Unpin>,
    session_clock: SessionClock,
    settings: CreationOptions,
}

impl<SessionClock, Rng> MessageScheduler<SessionClock, Rng> {
    pub const fn notify_new_data_message(&mut self) {
        self.cover_traffic.notify_new_data_message();
    }

    pub fn schedule_message(&mut self, message: OutboundMessage) {
        self.release_delayer.schedule_message(message);
    }
}

impl<SessionClock, Rng> Stream for MessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone + Unpin,
{
    type Item = RoundInfo;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            cover_traffic,
            release_delayer,
            round_clock,
            session_clock,
            settings,
        } = &mut *self;

        // We update session info on new sessions.
        if let Poll::Ready(Some(new_session_info)) = session_clock.poll_next_unpin(cx) {
            setup_new_session(
                cover_traffic,
                release_delayer,
                round_clock,
                settings,
                &new_session_info,
            );
        }

        // We do not return anything if a new round has not elapsed.
        let Poll::Ready(Some(())) = round_clock.poll_next_unpin(cx) else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        // We poll the sub-stream and return the right result accordingly.
        let cover_traffic_output = cover_traffic.poll_next_round().map(|()| vec![].into());
        let release_delayer_output = release_delayer.poll_next_round();

        match (cover_traffic_output, release_delayer_output) {
            // If none of the sub-streams yields a result, we do not return anything.
            (None, None) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            // If at least one sub-stream yields a result, we return a new element.
            (cover_message, processed_messages) => Poll::Ready(Some(RoundInfo {
                cover_message,
                processed_messages: processed_messages.unwrap_or_default(),
            })),
        }
    }
}

fn setup_new_session<Rng>(
    cover_traffic: &mut SessionCoverTraffic,
    release_delayer: &mut SessionReleaseDelayer<Rng>,
    round_clock: &mut Box<dyn Stream<Item = ()> + Unpin>,
    settings: &CreationOptions,
    new_session_info: &SessionInfo,
) where
    Rng: rand::Rng + Clone,
{
    *cover_traffic = settings.cover_traffic(new_session_info.core_quota, &mut release_delayer.rng);
    *release_delayer = settings.release_delayer(release_delayer.rng.clone());
    *round_clock = settings.round_clock();
}

#[derive(Debug, Clone, Copy)]
pub struct CreationOptions {
    pub additional_safety_intervals: usize,
    pub expected_intervals_per_session: NonZeroUsize,
    pub maximum_release_delay_in_rounds: NonZeroUsize,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroUsize,
}

impl CreationOptions {
    fn cover_traffic<Rng>(&self, core_quota: usize, rng: &mut Rng) -> SessionCoverTraffic
    where
        Rng: rand::Rng,
    {
        SessionCoverTraffic::new(
            crate::cover_traffic_2::CreationOptions {
                additional_safety_intervals: self.additional_safety_intervals,
                expected_intervals_per_session: self.expected_intervals_per_session,
                rounds_per_interval: self.rounds_per_interval,
                starting_quota: core_quota,
            },
            rng,
        )
    }

    fn release_delayer<Rng>(&self, rng: Rng) -> SessionReleaseDelayer<Rng>
    where
        Rng: rand::Rng,
    {
        SessionReleaseDelayer::new(
            crate::release_delayer::CreationOptions {
                maximum_release_delay_in_rounds: self.maximum_release_delay_in_rounds,
            },
            rng,
        )
    }

    fn round_clock(&self) -> Box<dyn Stream<Item = ()> + Unpin> {
        Box::new(IntervalStream::new(time::interval(self.round_duration)).map(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_substream_ready() {}

    #[test]
    fn cover_traffic_substream_ready() {}

    #[test]
    fn release_delayer_substream_ready() {}

    #[test]
    fn both_substreams_ready() {}
}
