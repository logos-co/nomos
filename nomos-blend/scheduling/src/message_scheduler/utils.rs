use fork_stream::StreamExt as _;
use futures::StreamExt as _;
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use tracing::trace;

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{LOG_TARGET, Settings, round_info::RoundClock, session_info::SessionInfo},
    release_clock::SessionReleaseClock,
};

/// Reset the sub-streams providing the new session info and the round clock at
/// the beginning of a new session.
pub(super) fn setup_new_session<Rng>(
    cover_traffic: &mut SessionCoverTraffic<RoundClock>,
    release_clock: &mut SessionReleaseClock<RoundClock, Rng>,
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
    *release_clock = instantiate_new_release_clock(
        rng,
        Box::new(round_clock_fork.clone()) as RoundClock,
        &settings,
    );
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

pub(super) fn instantiate_new_release_clock<Rng>(
    rng: Rng,
    round_clock: RoundClock,
    settings: &Settings,
) -> SessionReleaseClock<RoundClock, Rng>
where
    Rng: rand::Rng,
{
    SessionReleaseClock::new(
        crate::release_clock::Settings {
            maximum_release_delay_in_rounds: settings.maximum_release_delay_in_rounds,
        },
        rng,
        round_clock,
    )
}
