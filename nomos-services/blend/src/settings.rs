use std::{num::NonZeroU64, time::Duration};

use nomos_blend_message::crypto::Ed25519PublicKey;
use nomos_blend_scheduling::membership::{Membership, Node};
use serde::{Deserialize, Serialize};

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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MembershipSettings<NodeId>(Vec<Node<NodeId>>);

impl<NodeId> MembershipSettings<NodeId> {
    #[must_use]
    pub const fn new(nodes: Vec<Node<NodeId>>) -> Self {
        Self(nodes)
    }

    #[must_use]
    pub const fn size(&self) -> usize {
        self.0.len()
    }
}

impl<NodeId: Clone> MembershipSettings<NodeId> {
    #[must_use]
    pub fn membership(
        &self,
        local_signing_pubkey: Option<&Ed25519PublicKey>,
    ) -> Membership<NodeId> {
        Membership::new(&self.0, local_signing_pubkey)
    }
}
