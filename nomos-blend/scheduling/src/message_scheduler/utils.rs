use futures::{
    stream::{AbortHandle, Abortable},
    StreamExt as _,
};
use tokio::{
    sync::broadcast::{channel, Receiver},
    time::interval,
};
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tracing::{error, trace};

use crate::{
    cover_traffic::SessionCoverTraffic,
    message_scheduler::{
        round_info::{Round, RoundClock},
        session_info::SessionInfo,
        Settings, LOG_TARGET,
    },
    release_delayer::SessionProcessedMessageDelayer,
};

/// Reset the sub-streams providing the new session info and the round clock at
/// the beginning of a new session.
pub(super) fn setup_new_session<Rng, ProcessedMessage>(
    cover_traffic: &mut SessionCoverTraffic<RoundClock>,
    release_delayer: &mut SessionProcessedMessageDelayer<RoundClock, Rng, ProcessedMessage>,
    round_clock: &mut RoundClock,
    round_clock_task_abort_handle: &mut AbortHandle,
    settings: Settings,
    mut rng: Rng,
    new_session_info: SessionInfo,
) where
    Rng: rand::Rng,
{
    trace!(target: LOG_TARGET, "New session {} started with session info: {new_session_info:?}", new_session_info.session_number);
    kill_round_clock(round_clock_task_abort_handle);

    let mut new_round_clock = IntervalStream::new(interval(settings.round_duration))
        .enumerate()
        .map(|(round, _)| Some((round as u128).into()));
    let (round_clock_stream_sender, _) = channel(3);
    let round_clock_stream_sender_clone = round_clock_stream_sender.clone();

    // Spawn a task that sends ticks to all the receiving streams.
    let (new_abort_handle, new_abort_registration) = AbortHandle::new_pair();
    tokio::spawn(Abortable::new(
        async move {
            while let Some(new_round) = new_round_clock.next().await {
                if let Err(send_error) = round_clock_stream_sender_clone.send(new_round) {
                    error!(target: LOG_TARGET, "Failed to send round tick to consumers. Error: {send_error:?}");
                }
            }
            trace!(target: LOG_TARGET, "Round clock terminated.");
        },
        new_abort_registration,
    ));

    *cover_traffic = instantiate_new_cover_scheduler(
        &mut rng,
        round_clock_stream_sender.subscribe(),
        &settings,
        new_session_info.core_quota,
    );
    *release_delayer =
        instantiate_new_message_delayer(rng, round_clock_stream_sender.subscribe(), &settings);
    *round_clock = get_round_clock(round_clock_stream_sender.subscribe());
    *round_clock_task_abort_handle = new_abort_handle;
}

pub(super) fn kill_round_clock(round_clock_abort_handle: &AbortHandle) {
    round_clock_abort_handle.abort();
    while !round_clock_abort_handle.is_aborted() {}
}

pub(super) fn instantiate_new_cover_scheduler<Rng>(
    rng: &mut Rng,
    round_clock_stream_receiver: Receiver<Option<Round>>,
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
        get_round_clock(round_clock_stream_receiver),
    )
}

pub(super) fn instantiate_new_message_delayer<Rng, ProcessedMessage>(
    rng: Rng,
    round_clock_stream_receiver: Receiver<Option<Round>>,
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
        get_round_clock(round_clock_stream_receiver),
    )
}

fn get_round_clock(stream_receiver: Receiver<Option<Round>>) -> RoundClock {
    Box::new(BroadcastStream::new(stream_receiver).map(|round| {
        round
            .expect("Round to be `Ok`.")
            .expect("Round to be `Some`.")
    })) as RoundClock
}
