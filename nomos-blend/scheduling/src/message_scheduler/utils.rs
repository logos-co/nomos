use fork_stream::StreamExt as _;
use futures::StreamExt as _;
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use tracing::trace;

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{round_info::RoundClock, session_info::SessionInfo, Settings, LOG_TARGET},
    release_delayer::SessionProcessedMessageDelayer,
};

/// Reset the sub-streams providing the new session info and the round clock at
/// the beginning of a new session.
pub(super) fn setup_new_session<Rng, ProcessedMessage>(
    cover_traffic: &mut SessionCoverTraffic<RoundClock>,
    release_delayer: &mut SessionProcessedMessageDelayer<RoundClock, Rng, ProcessedMessage>,
    round_clock: &mut RoundClock,
    settings: Settings,
    mut rng: Rng,
    new_session_info: SessionInfo,
) where
    Rng: rand::Rng,
{
    trace!(target: LOG_TARGET, "New session {} started with session info: {new_session_info:?}", new_session_info.session_number);

    let new_round_clock = Box::new(
        IntervalStream::new(interval(settings.round_duration))
            .enumerate()
            .map(|(round, _)| (round as u128).into()),
    ) as RoundClock;
    let round_clock_fork = new_round_clock.fork();

    *cover_traffic = instantiate_new_cover_scheduler(
        &mut rng,
        Box::new(round_clock_fork.clone()) as RoundClock,
        &settings,
        new_session_info.core_quota,
    );
    *release_delayer = instantiate_new_message_delayer(
        rng,
        Box::new(round_clock_fork.clone()) as RoundClock,
        &settings,
    );
    *round_clock = Box::new(round_clock_fork) as RoundClock;
}

pub(super) fn instantiate_new_cover_scheduler<Rng>(
    rng: &mut Rng,
    round_clock: RoundClock,
    settings: &Settings,
    starting_quota: u64,
) -> SessionCoverTraffic<RoundClock>
where
    Rng: rand::Rng,
{
    SessionCoverTraffic::new(
        crate::cover_traffic::Settings {
            additional_safety_intervals: settings.additional_safety_intervals,
            expected_intervals_per_session: settings.expected_intervals_per_session,
            rounds_per_interval: settings.rounds_per_interval,
            starting_quota,
        },
        rng,
        round_clock,
    )
}

pub(super) fn instantiate_new_message_delayer<Rng, ProcessedMessage>(
    rng: Rng,
    round_clock: RoundClock,
    settings: &Settings,
) -> SessionProcessedMessageDelayer<RoundClock, Rng, ProcessedMessage>
where
    Rng: rand::Rng,
{
    SessionProcessedMessageDelayer::new(
        crate::release_delayer::Settings {
            maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
        },
        rng,
        round_clock,
    )
}
