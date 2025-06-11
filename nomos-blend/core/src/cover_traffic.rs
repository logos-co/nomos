use std::{
    collections::HashSet,
    fmt::{Debug, Display, Formatter},
    num::NonZeroU64,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{stream::empty, Stream, StreamExt as _};
use nomos_utils::math::NonNegativeF64;
use tokio::{sync::mpsc, time};
use tokio_stream::wrappers::IntervalStream;
use tracing::{debug, error, trace, warn};

// max: safety buffer length, expressed in intervals
const SAFETY_BUFFER_INTERVALS: u64 = 100;
const LOG_TARGET: &str = "blend::core::cover";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Round(u64);

impl From<u64> for Round {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Display for Round {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Interval(u64);

impl From<u64> for Interval {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Display for Interval {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Session(u64);

impl From<u64> for Session {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Display for Session {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone)]
pub struct CoverTrafficSettings {
    /// `S`: length of a session in terms of expected rounds (on average).
    pub rounds_per_session: NonZeroU64,
    /// `|I|`: length of an interval in terms of rounds.
    pub rounds_per_interval: NonZeroU64,
    /// Duration of a round.
    pub round_duration: Duration,
    /// `F_c`: frequency at which cover messages are generated per round.
    pub message_frequency_per_round: NonNegativeF64,
    /// `ß_c`: expected number of blending operations for each cover message.
    pub blending_ops_per_message: usize,
    /// `R_c`: redundancy parameter for cover messages.
    pub redundancy_parameter: usize,
}

impl CoverTrafficSettings {
    #[must_use]
    pub const fn intervals_per_session(&self) -> u64 {
        self.rounds_per_session
            .get()
            .checked_div(self.rounds_per_interval.get())
            .expect("Calculating the number of intervals per session failed.")
    }

    #[must_use]
    pub const fn intervals_per_session_including_safety_buffer(&self) -> u64 {
        self.intervals_per_session().checked_add(SAFETY_BUFFER_INTERVALS).expect("Overflow when calculating the total number of intervals for the session, including the safety buffer.")
    }
}

/// Information computed internally at every session change.
#[derive(Debug, Clone, Default)]
struct InternalSessionInfo {
    // Used mark rounds that should result in a new cover message, for a given session.
    message_slots: HashSet<(Interval, Round)>,
    // The current session number.
    session_number: Session,
}

impl InternalSessionInfo {
    // TODO: Remove unsafe casts
    /// Given the new session info and the cover message settings, it computes
    /// the new maximum quota as per the spec, and randomly generated rounds at
    /// which such quota will be depleted.
    fn from_session_info_and_settings<Rng>(
        session_info: &SessionInfo,
        mut rng: Rng,
        settings: &CoverTrafficSettings,
    ) -> Self
    where
        Rng: rand::Rng,
    {
        // C: Expected number of cover messages that are generated during a session.
        let expected_number_of_session_messages =
            settings.rounds_per_session.get() as f64 * settings.message_frequency_per_round.get();
        // Q_c: Messaging allowance that can be used by a core node during a
        // single session.
        let core_quota = ((expected_number_of_session_messages
            * (settings.blending_ops_per_message
                + settings.redundancy_parameter * settings.blending_ops_per_message)
                as f64)
            / session_info.membership_size as f64)
            .ceil();
        // c: Maximal number of cover messages a node can generate per session.
        let session_messages =
            (core_quota / settings.blending_ops_per_message as f64).ceil() as usize;

        let message_slots = generate_message_slots(session_messages, settings, &mut rng);

        Self {
            message_slots,
            session_number: session_info.session_number,
        }
    }
}

/// As per the spec, it randomly generates rounds at which a cover message will
/// be generated, over the whole duration of the session, including the
/// specified safety buffer intervals.
fn generate_message_slots<Rng>(
    mut total_message_count: usize,
    settings: &CoverTrafficSettings,
    rng: &mut Rng,
) -> HashSet<(Interval, Round)>
where
    Rng: rand::Rng,
{
    let mut message_slots = HashSet::with_capacity(total_message_count);
    trace!(target: LOG_TARGET, "Generating {total_message_count} cover message slots.");
    while total_message_count > 0 {
        // Pick a random interval.
        let random_interval =
            rng.gen_range(0..settings.intervals_per_session_including_safety_buffer());
        // Pick a random round within that interval
        let random_round = rng.gen_range(0..settings.rounds_per_interval.get());
        // Add it to the pre-computed slots. If an entry exists, do nothing and try
        // again.
        if message_slots.insert((random_interval.into(), random_round.into())) {
            total_message_count -= 1;
        } else {
            trace!(target: LOG_TARGET, "Random round generation generated an existing entry. Retrying...");
        }
    }
    message_slots
}

/// Information that the input stream to this module must provide at every
/// session change.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// The size of the list of core nodes participating in Blend.
    pub membership_size: usize,
    /// The current session number.
    pub session_number: Session,
}

pub struct CoverTraffic<SessionStream, Rng> {
    /// The session stream triggered at every session change, provided from the
    /// outside.
    sessions: SessionStream,
    /// The internal stream that generates a new element at every round change.
    rounds: Box<dyn Stream<Item = Round> + Send + Unpin>,
    /// The internal stream that generates a new element at every interval.
    intervals: Box<dyn Stream<Item = Interval> + Send + Unpin>,
    /// The current interval value, used to look into the pre-generated slots.
    current_interval: Interval,
    /// The provided settings.
    settings: CoverTrafficSettings,
    /// The info corresponding to the currently running session.
    session_info: InternalSessionInfo,
    /// The RNG to pre-compute new slots at every session change.
    rng: Rng,
    /// The channel to notify this module about data messages the node has
    /// already sent, so that it can modify its own scheduling accordingly, as
    /// per the spec.
    data_message_emission_notification_channel: (mpsc::Sender<()>, mpsc::Receiver<()>),
    waker: Option<Waker>,
}

impl<SessionStream, Rng> CoverTraffic<SessionStream, Rng> {
    pub fn new(settings: CoverTrafficSettings, sessions: SessionStream, rng: Rng) -> Self {
        Self {
            // We don't start any timers until we get information about a session.
            rounds: Box::new(empty::<Round>()),
            intervals: Box::new(empty::<Interval>()),
            current_interval: Interval::default(),
            sessions,
            settings,
            session_info: InternalSessionInfo::default(),
            rng,
            // Channel is created assuming the node might generate a message for each round, for
            // each interval, including the safety buffer. This is the absolutely worst case.
            data_message_emission_notification_channel: mpsc::channel(
                (settings.intervals_per_session_including_safety_buffer()
                    * settings.rounds_per_session.get()) as usize,
            ),
            waker: None,
        }
    }

    pub async fn notify_of_new_data_message(&mut self) {
        if let Err(e) = self
            .data_message_emission_notification_channel
            .0
            .send(())
            .await
        {
            error!(target: LOG_TARGET, "Failed to notify cover message stream of new data message generated. Error = {e:?}");
        }
    }
}

fn rounds_stream(round_duration: Duration) -> Box<dyn Stream<Item = Round> + Send + Unpin> {
    Box::new(
        IntervalStream::new(time::interval(round_duration))
            .enumerate()
            .map(move |(i, _)| (i as u64).into()),
    )
}

fn intervals_stream(
    interval_duration: Duration,
) -> Box<dyn Stream<Item = Interval> + Send + Unpin> {
    Box::new(
        IntervalStream::new(time::interval(interval_duration))
            .enumerate()
            .map(move |(i, _)| (i as u64).into()),
    )
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
            waker,
        } = &mut *self;

        if let Poll::Ready(Some(new_session_info)) = sessions.poll_next_unpin(cx) {
            on_session_change(session_info, &new_session_info, rng, settings, intervals);
        }
        if let Poll::Ready(Some(new_interval)) = intervals.poll_next_unpin(cx) {
            on_interval_change(
                session_info,
                current_interval,
                new_interval,
                settings,
                rounds,
            );
        }
        let res = if let Poll::Ready(Some(new_round)) = rounds.poll_next_unpin(cx) {
            if let Some(w) = waker.take() {
                w.wake();
            }
            on_new_round(
                session_info,
                *current_interval,
                new_round,
                cx,
                &mut data_message_emission_notification_channel.1,
            )
        } else {
            *waker = Some(cx.waker().clone());
            Poll::Pending
        };
        res
    }
}

fn on_session_change<Rng>(
    current_session_info: &mut InternalSessionInfo,
    new_session_info: &SessionInfo,
    rng: Rng,
    settings: &CoverTrafficSettings,
    intervals: &mut Box<dyn Stream<Item = Interval> + Send + Unpin>,
) where
    Rng: rand::Rng,
{
    debug!(target: LOG_TARGET, "Session {} started.", new_session_info.session_number);
    *current_session_info =
        InternalSessionInfo::from_session_info_and_settings(new_session_info, rng, settings);
    *intervals = intervals_stream(Duration::from_secs(
        settings.rounds_per_interval.get() * settings.round_duration.as_secs(),
    ));
}

fn on_interval_change(
    current_session_info: &InternalSessionInfo,
    current_interval: &mut Interval,
    new_interval: Interval,
    settings: &CoverTrafficSettings,
    rounds: &mut Box<dyn Stream<Item = Round> + Send + Unpin>,
) {
    let maximum_interval_value = settings.intervals_per_session_including_safety_buffer();
    if new_interval < Interval::from(maximum_interval_value) {
        *current_interval = new_interval;
        *rounds = rounds_stream(settings.round_duration);
        debug!(
            target: LOG_TARGET, "Interval {new_interval} started for session {}.",
            current_session_info.session_number
        );
    } else {
        warn!(target: LOG_TARGET, "Interval stream has passed the expected limit including the safety buffer. Current value = {new_interval:?}, maximum allowed = {maximum_interval_value:?}");
    }
}

fn on_new_round(
    current_session_info: &mut InternalSessionInfo,
    current_interval: Interval,
    new_round: Round,
    poll_context: &mut Context,
    data_message_receiver_channel: &mut mpsc::Receiver<()>,
) -> Poll<Option<Vec<u8>>> {
    debug!(
        target: LOG_TARGET, "New round {new_round} started for interval {current_interval} and session {}.",
        current_session_info.session_number
    );
    let should_emit_scheduled_cover_message = current_session_info
        .message_slots
        .remove(&(current_interval, new_round));
    // If we have not scheduled to emit a new cover message, we do not consume the
    // incoming channel at all.
    if !should_emit_scheduled_cover_message {
        trace!(target: LOG_TARGET, "Not a pre-scheduled emission for this round.");
        return Poll::Pending;
    }

    let data_message_override = matches!(
        data_message_receiver_channel.poll_recv(poll_context),
        Poll::Ready(Some(()))
    );
    if data_message_override {
        trace!(target: LOG_TARGET, "Skipping message emission because of override by data message.");
        Poll::Pending
    } else {
        debug!(
            target: LOG_TARGET, "Emitting new cover message for (interval, round) ({current_interval}, {new_round})"
        );
        Poll::Ready(Some(vec![]))
    }
}

#[cfg(test)]
mod tests {
    use core::{num::NonZeroU64, time::Duration};

    use futures::StreamExt as _;
    use rand::SeedableRng as _;
    use rand_chacha::ChaCha12Rng;
    use tokio::time;
    use tokio_stream::wrappers::IntervalStream;

    use crate::cover_traffic::{CoverTraffic, CoverTrafficSettings, SessionInfo};

    #[test_log::test(tokio::test)]
    async fn message_emission_without_data_messages() {
        let settings = CoverTrafficSettings {
            blending_ops_per_message: 3,
            message_frequency_per_round: 1f64.try_into().unwrap(),
            redundancy_parameter: 0,
            round_duration: Duration::from_secs(1),
            rounds_per_interval: NonZeroU64::try_from(3).unwrap(),
            rounds_per_session: NonZeroU64::try_from(9).unwrap(),
        };
        let mut cover_traffic = CoverTraffic::new(
            settings,
            Box::new(
                IntervalStream::new(time::interval(Duration::from_secs(
                    settings.round_duration.as_secs() * settings.rounds_per_session.get(),
                )))
                .enumerate()
                .map(|(i, _)| SessionInfo {
                    membership_size: 1,
                    session_number: (i as u64).into(),
                }),
            ),
            ChaCha12Rng::from_entropy(),
        );

        loop {
            tokio::select! {
                Some(msg) = cover_traffic.next() => {
                    println!("Next: {msg:?}");
                },
            }
        }
    }

    #[test_log::test(tokio::test)]
    async fn message_emission_with_data_messages() {}

    #[test_log::test(tokio::test)]
    async fn message_emission_with_long_session() {}
}
