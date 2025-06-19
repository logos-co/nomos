use std::{
    num::NonZeroUsize,
    pin::{pin, Pin},
    task::{Context, Poll},
    time::Duration,
};

use futures::{Stream, StreamExt as _};

use crate::{
    cover_traffic_2::SessionCoverTraffic,
    message_scheduler::{round_info::RoundInfo, session_info::SessionInfo},
    release_delayer::SessionReleaseDelayer,
};

pub mod round_info;
pub mod session_info;

pub struct UninitializedMessageScheduler<SessionClock, Rng> {
    session_clock: SessionClock,
    settings: CreationOptions<Rng>,
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng> {
    pub fn new(session_clock: SessionClock, settings: CreationOptions<Rng>) -> Self {
        Self {
            session_clock,
            settings,
        }
    }
}

impl<SessionClock, Rng> UninitializedMessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Clone,
{
    pub async fn wait_ready(self) -> MessageScheduler<SessionClock, Rng> {
        let Self {
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

        let core_quota = first_session_info.core_quota;
        MessageScheduler {
            current_session_info: first_session_info,
            cover_traffic: SessionCoverTraffic::new(crate::cover_traffic_2::CreationOptions {
                additional_safety_intervals: settings.additional_safety_intervals,
                expected_intervals_per_session: settings.expected_intervals_per_session,
                rng: settings.rng.clone(),
                rounds_per_interval: settings.rounds_per_interval,
                starting_quota: core_quota,
            }),
            release_delayer: SessionReleaseDelayer::new(crate::release_delayer::CreationOptions {
                maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
                rng: settings.rng,
            }),
            session_clock,
        }
    }
}

pub struct MessageScheduler<SessionClock, Rng> {
    current_session_info: SessionInfo,
    cover_traffic: SessionCoverTraffic,
    release_delayer: SessionReleaseDelayer<Rng>,
    round_clock: Box<dyn Stream<Item = ()>>,
    session_clock: SessionClock,
}

impl<SessionClock, Rng> Stream for MessageScheduler<SessionClock, Rng>
where
    SessionClock: Stream<Item = SessionInfo>,
{
    type Item = RoundInfo;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unimplemented!()
    }
}

pub struct CreationOptions<Rng> {
    pub additional_safety_intervals: usize,
    pub expected_intervals_per_session: NonZeroUsize,
    pub maximum_release_delay_in_rounds: NonZeroUsize,
    pub rng: Rng,
    pub round_duration: Duration,
    pub rounds_per_interval: NonZeroUsize,
}
