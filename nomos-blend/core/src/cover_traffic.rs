use std::{
    collections::HashSet,
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Stream, StreamExt as _};
use tokio::{sync::mpsc, time};
use tokio_stream::wrappers::IntervalStream;
use tracing::{debug, error, trace};

// max: safety buffer length, expressed in intervals
const SAFETY_BUFFER_INTERVALS: u64 = 100;

#[derive(Copy, Clone)]
pub struct CoverTrafficSettings {
    // length of a session in terms of expected intervals.
    intervals_per_session: u64,
    // |I|: length of an interval in terms of rounds.
    rounds_per_interval: u64,
    // length of a round in terms of seconds.
    round_duration: Duration,
    // F_c: frequency at which cover messages are generated per round.
    message_frequency_per_round: f64,
    // H_c: expected number of blending operations for each cover message.
    blending_ops_per_message: usize,
    // R_c: redundancy parameter for cover messages.
    redundancy_parameter: usize,
    // ß_max: maximum number of blending operations of a single message.
    maximum_blending_ops_per_message: usize,
}

impl CoverTrafficSettings {
    #[must_use]
    pub const fn new(
        intervals_per_session: u64,
        rounds_per_interval: u64,
        round_duration: Duration,
        message_frequency_per_round: f64,
        blending_ops_per_message: usize,
        redundancy_parameter: usize,
        maximum_blending_ops_per_message: usize,
    ) -> Self {
        assert!(
            message_frequency_per_round >= 0f64,
            "Message frequency per round cannot have a negative value."
        );
        Self {
            intervals_per_session,
            rounds_per_interval,
            round_duration,
            message_frequency_per_round,
            blending_ops_per_message,
            redundancy_parameter,
            maximum_blending_ops_per_message,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct InternalSessionInfo {
    // Used mark rounds that should result in a new cover message, for a given session.
    // The key is (interval, round).
    message_slots: HashSet<(u64, u64)>,
    session_number: u64,
}

impl InternalSessionInfo {
    // TODO: Remove unsafe casts
    fn from_session_info_and_settings<Rng>(
        session_info: &SessionInfo,
        mut rng: Rng,
        &CoverTrafficSettings {
            blending_ops_per_message,
            message_frequency_per_round,
            redundancy_parameter,
            maximum_blending_ops_per_message,
            intervals_per_session,
            rounds_per_interval,
            ..
        }: &CoverTrafficSettings,
    ) -> Self
    where
        Rng: rand::Rng,
    {
        let rounds_per_session = rounds_per_interval * intervals_per_session;
        // C: Expected number of cover messages that are generated during a session.
        let expected_number_of_session_messages =
            rounds_per_session as f64 * message_frequency_per_round;
        // Q_c: Messaging allowance that can be used by a core node during a
        // single session.
        let core_quota = ((expected_number_of_session_messages
            * (blending_ops_per_message + redundancy_parameter * blending_ops_per_message) as f64)
            / session_info.membership_size as f64)
            .ceil();
        // c: Maximal number of cover messages a node can generate per session.
        let mut session_messages =
            (core_quota / maximum_blending_ops_per_message as f64).ceil() as usize;

        let mut message_slots = HashSet::with_capacity(session_messages);
        let total_intervals =
            (rounds_per_session / intervals_per_session) + SAFETY_BUFFER_INTERVALS;
        while session_messages > 0 {
            let random_interval = rng.gen_range(0..total_intervals);
            let random_round = rng.gen_range(0..intervals_per_session);
            if message_slots.insert((random_interval, random_round)) {
                session_messages -= 1;
            } else {
                trace!("Random slot generation generated an existing entry. Retrying...");
            }
        }

        Self {
            message_slots,
            session_number: session_info.session_number,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub membership_size: usize,
    pub session_number: u64,
}

pub struct CoverTraffic<SessionStream, Rng> {
    sessions: SessionStream,
    rounds: Box<dyn Stream<Item = u64> + Send + Unpin>,
    intervals: Box<dyn Stream<Item = u64> + Send + Unpin>,
    current_interval: u64,
    settings: CoverTrafficSettings,
    session_info: InternalSessionInfo,
    rng: Rng,
    data_message_emission_notification_channel: (mpsc::Sender<()>, mpsc::Receiver<()>),
}

impl<SessionStream, Rng> CoverTraffic<SessionStream, Rng> {
    pub fn new(settings: CoverTrafficSettings, sessions: SessionStream, rng: Rng) -> Self {
        let rounds = Box::new(
            IntervalStream::new(time::interval(settings.round_duration))
                .enumerate()
                .map(move |(i, _)| i as u64 % settings.rounds_per_interval),
        );
        let intervals = Box::new(
            IntervalStream::new(time::interval(Duration::from_secs(
                settings.rounds_per_interval * settings.round_duration.as_secs(),
            )))
            .enumerate()
            .map(move |(i, _)| i as u64 % settings.intervals_per_session),
        );
        Self {
            rounds,
            intervals,
            current_interval: 0,
            sessions,
            settings,
            session_info: InternalSessionInfo::default(),
            rng,
            data_message_emission_notification_channel: mpsc::channel(
                (settings.rounds_per_interval * settings.intervals_per_session) as usize,
            ),
        }
    }

    pub async fn notify_new_data_message(&mut self) {
        if let Err(e) = self
            .data_message_emission_notification_channel
            .0
            .send(())
            .await
        {
            error!("Failed to notify cover message stream of new data message generated. Error = {e:?}");
        }
    }
}

impl<SessionStream, Rng> Stream for CoverTraffic<SessionStream, Rng>
where
    SessionStream: Stream<Item = SessionInfo> + Unpin,
    Rng: rand::Rng + Unpin,
{
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            settings,
            rng,
            session_info,
            current_interval,
            sessions,
            rounds,
            intervals,
            data_message_emission_notification_channel,
        } = &mut *self;

        if let Poll::Ready(Some(new_session_info)) = sessions.poll_next_unpin(cx) {
            on_session_change(session_info, &new_session_info, rng, settings);
        }
        if let Poll::Ready(Some(new_interval)) = intervals.poll_next_unpin(cx) {
            on_interval_change(session_info, current_interval, new_interval);
        }
        if let Poll::Ready(Some(new_round)) = rounds.poll_next_unpin(cx) {
            if on_new_round(
                session_info,
                *current_interval,
                new_round,
                cx,
                &mut data_message_emission_notification_channel.1,
            ) == Some(())
            {
                return Poll::Ready(Some(vec![]));
            }
        }
        Poll::Pending
    }
}

fn on_session_change<Rng>(
    current_session_info: &mut InternalSessionInfo,
    new_session_info: &SessionInfo,
    rng: Rng,
    settings: &CoverTrafficSettings,
) where
    Rng: rand::Rng,
{
    debug!("Session {} started.", new_session_info.session_number);
    *current_session_info =
        InternalSessionInfo::from_session_info_and_settings(new_session_info, rng, settings);
}

fn on_interval_change(
    current_session_info: &InternalSessionInfo,
    current_interval: &mut u64,
    new_interval: u64,
) {
    *current_interval = new_interval;
    debug!(
        "Interval {new_interval} started for session {}.",
        current_session_info.session_number
    );
}

fn on_new_round(
    current_session_info: &mut InternalSessionInfo,
    current_interval: u64,
    new_round: u64,
    poll_context: &mut Context,
    data_message_receiver_channel: &mut mpsc::Receiver<()>,
) -> Option<()> {
    debug!(
        "New round {new_round} started for interval {} and session {}.",
        current_interval, current_session_info.session_number
    );
    let should_emit_scheduled_cover_message = current_session_info
        .message_slots
        .remove(&(current_interval, new_round));
    let data_message_override = matches!(
        data_message_receiver_channel.poll_recv(poll_context),
        Poll::Ready(Some(()))
    );
    if should_emit_scheduled_cover_message && !data_message_override {
        return Some(());
    }
    None
}
