use std::{
    collections::HashSet,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    stream::{Enumerate, Map},
    Stream, StreamExt as _,
};
use multiaddr::PeerId;
use serde::Deserialize;
use tokio::time::{self, Instant};
use tokio_stream::wrappers::IntervalStream;
use tracing::trace;

#[derive(Copy, Clone, Deserialize)]
pub struct CoverTrafficSettings {
    // S: number of rounds in a session.
    rounds_per_session: u64,
    // |I|: length of an interval in terms of rounds.
    interval_duration: u64,
    // length of a round in terms of seconds.
    round_duration: u64,
    // F_c: frequency at which cover messages are generated per round.
    // TODO: Prevent this from being negative
    message_frequency_per_round: f64,
    // H_c: expected number of blending operations for each cover message.
    blending_ops_per_message: usize,
    // R_c: redundancy parameter for cover messages.
    redundancy_parameter: usize,
    // ß_max: maximum number of blending operations of a single message.
    maximum_blending_ops_per_message: usize,
    // max: safety buffer length, expressed in intervals
    safety_buffer_intervals: u64,
}

impl CoverTrafficSettings {
    fn rounds_stream(&self) -> Map<Enumerate<IntervalStream>, impl FnMut((usize, Instant)) -> u64> {
        IntervalStream::new(time::interval(Duration::from_secs(self.round_duration)))
            .enumerate()
            .map(|(i, _)| i as u64)
    }

    fn intervals_stream(
        &self,
    ) -> Map<Enumerate<IntervalStream>, impl FnMut((usize, Instant)) -> u64> {
        IntervalStream::new(time::interval(Duration::from_secs(
            self.interval_duration * self.round_duration,
        )))
        .enumerate()
        .map(|(i, _)| i as u64)
    }
}

#[derive(Debug, Clone, Default)]
struct InternalSessionInfo {
    // Used mark rounds that should result in a new cover message.
    // The key is (interval, interval).
    message_slots: HashSet<(u64, u64)>,
    current_interval: u64,
}

impl InternalSessionInfo {
    // TODO: Remove unsafe casts
    fn from_session_info_and_settings<Rng>(
        session_info: SessionInfo,
        mut rng: Rng,
        &CoverTrafficSettings {
            blending_ops_per_message,
            message_frequency_per_round,
            redundancy_parameter,
            maximum_blending_ops_per_message,
            rounds_per_session,
            interval_duration,
            safety_buffer_intervals: safety_buffer_length,
            ..
        }: &CoverTrafficSettings,
    ) -> Self
    where
        Rng: rand::Rng,
    {
        // C: Expected number of cover messages that are generated during a session.
        let expected_number_of_session_messages =
            rounds_per_session as f64 * message_frequency_per_round;
        // Q_c: Messaging allowance that can be used by a core node during a
        // single session.
        let core_quota = ((expected_number_of_session_messages
            * (blending_ops_per_message + redundancy_parameter * blending_ops_per_message) as f64)
            / session_info.core_nodes.len() as f64)
            .ceil();
        // c: Maximal number of cover messages a node can generate per session.
        let mut session_messages =
            (core_quota / maximum_blending_ops_per_message as f64).ceil() as usize;

        let mut message_slots = HashSet::with_capacity(session_messages);
        let total_intervals = (rounds_per_session / interval_duration) + safety_buffer_length;
        while session_messages > 0 {
            let random_interval = rng.gen_range(0..total_intervals);
            let random_round = rng.gen_range(0..interval_duration);
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
    core_nodes: Vec<PeerId>,
}

pub struct CoverTraffic<SessionStream, Rng> {
    sessions: SessionStream,
    rounds: Box<dyn Stream<Item = u64> + Unpin>,
    intervals: Box<dyn Stream<Item = u64> + Unpin>,
    settings: CoverTrafficSettings,
    session_info: InternalSessionInfo,
    rng: Rng,
}

impl<SessionStream, Rng> CoverTraffic<SessionStream, Rng> {
    pub fn new(settings: CoverTrafficSettings, sessions: SessionStream, rng: Rng) -> Self {
        Self {
            rounds: Box::new(settings.rounds_stream()),
            intervals: Box::new(settings.intervals_stream()),
            sessions,
            settings,
            session_info: InternalSessionInfo::default(),
            rng,
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
            sessions,
            rounds,
            intervals,
            ..
        } = &mut *self;

        // It's fine for multiple streams to tick at the same time, it means we enter a
        // new period (e.g., a new interval, or a new session), and all lower-level
        // streams should be reset. Before resetting, we need to fetch a value from them
        // in case they have one.
        // Maybe we can have our own implementation of `Stream` which returns what we
        // need?
        let should_emit_new_message =
            if let Poll::Ready(Some(new_round)) = rounds.poll_next_unpin(cx) {
                if session_info
                    .message_slots
                    .remove(&(session_info.current_interval, new_round))
                {
                    Some(true)
                } else {
                    Some(false)
                }
            } else {
                None
            };
    }
}
