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
use tracing::{error, trace};

// max: safety buffer length, expressed in intervals
const SAFETY_BUFFER_INTERVALS: u64 = 100;

#[derive(Copy, Clone)]
pub struct CoverTrafficSettings {
    // length of a session in terms of expected intervals.
    pub intervals_per_session: u64,
    // |I|: length of an interval in terms of rounds.
    pub rounds_per_interval: u64,
    // length of a round in terms of seconds.
    pub round_duration: Duration,
    // F_c: frequency at which cover messages are generated per round.
    // TODO: Prevent this from being negative
    pub message_frequency_per_round: f64,
    // H_c: expected number of blending operations for each cover message.
    pub blending_ops_per_message: usize,
    // R_c: redundancy parameter for cover messages.
    pub redundancy_parameter: usize,
    // ß_max: maximum number of blending operations of a single message.
    pub maximum_blending_ops_per_message: usize,
}

#[derive(Debug, Clone, Default)]
struct InternalSessionInfo {
    // Used mark rounds that should result in a new cover message.
    // The key is (interval, interval).
    message_slots: HashSet<(u64, u64)>,
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

        Self { message_slots }
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub membership_size: usize,
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
    SessionStream: Stream<Item = (usize, SessionInfo)> + Unpin,
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

        if let Poll::Ready(Some((_, new_session_info))) = sessions.poll_next_unpin(cx) {
            *session_info = InternalSessionInfo::from_session_info_and_settings(
                &new_session_info,
                rng,
                settings,
            );
        }
        if let Poll::Ready(Some(new_interval)) = intervals.poll_next_unpin(cx) {
            *current_interval = new_interval;
        }
        if let Poll::Ready(Some(new_round)) = rounds.poll_next_unpin(cx) {
            let should_emit_scheduled_cover_message = session_info
                .message_slots
                .remove(&(*current_interval, new_round));
            let data_message_override = matches!(
                data_message_emission_notification_channel.1.poll_recv(cx),
                Poll::Ready(Some(()))
            );
            if should_emit_scheduled_cover_message && !data_message_override {
                return Poll::Ready(Some(vec![]));
            }
        }
        Poll::Pending
    }
}
