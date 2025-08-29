use std::{hash::Hash, num::NonZeroU64, time::Duration};

use futures::{Stream, StreamExt as _};
use nomos_blend_scheduling::{
    membership::{Membership, Node},
    message_blend::CryptographicProcessorSettings,
};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::IntervalStream;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings<NodeId> {
    pub crypto: CryptographicProcessorSettings,
    pub time: TimingSettings,
    pub minimal_network_size: NonZeroU64,
    // TODO: Replace with SDP membership stream.
    //       We keep this for now since the membership service returns nothing.
    pub membership: Vec<Node<NodeId>>,
}

impl<NodeId> Settings<NodeId>
where
    NodeId: Eq + Hash + Clone,
{
    pub(crate) fn membership(&self) -> Membership<NodeId> {
        let local_signing_pubkey = self.crypto.signing_private_key.public_key();
        Membership::new(&self.membership, &local_signing_pubkey)
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TimingSettings {
    /// `S`: length of a session in terms of expected rounds (on average).
    pub rounds_per_session: NonZeroU64,
    /// `|I|`: length of an interval in terms of rounds.
    pub rounds_per_interval: NonZeroU64,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    /// Duration of a round.
    pub round_duration: Duration,
    pub rounds_per_observation_window: NonZeroU64,
    /// Session transition period in rounds.
    pub rounds_per_session_transition_period: NonZeroU64,
}

impl TimingSettings {
    #[must_use]
    pub const fn session_duration(&self) -> Duration {
        Duration::from_secs(self.rounds_per_session.get() * self.round_duration.as_secs())
    }

    #[must_use]
    pub fn intervals_per_session(&self) -> NonZeroU64 {
        NonZeroU64::try_from(self.rounds_per_session.get() / self.rounds_per_interval.get()).expect("Obtained `0` when calculating the number of intervals per session, which is not allowed.")
    }

    #[must_use]
    pub const fn session_transition_period(&self) -> Duration {
        Duration::from_secs(
            self.rounds_per_session_transition_period.get() * self.round_duration.as_secs(),
        )
    }
}

/// A stream that repeatedly yields the same membership at fixed intervals.
/// The first item is yielded immediately.
// TODO: Replace with SDP membership stream.
pub(crate) fn constant_membership_stream<NodeId>(
    membership: Membership<NodeId>,
    interval: Duration,
) -> impl Stream<Item = Membership<NodeId>>
where
    NodeId: Clone,
{
    IntervalStream::new(tokio::time::interval(interval)).map(move |_| membership.clone())
}
