use core::{
    fmt,
    fmt::{Display, Formatter},
};

use futures::Stream;

use crate::message_scheduler::message_queue::MessageBatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Round(u128);

impl Round {
    #[must_use]
    pub const fn inner(&self) -> u128 {
        self.0
    }
}

impl From<u128> for Round {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<Round> for u128 {
    fn from(round: Round) -> Self {
        round.0
    }
}

impl Display for Round {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub type RoundInfo<ProcessedMessage> = MessageBatch<ProcessedMessage>;
pub type RoundClock = Box<dyn Stream<Item = Round> + Send + Unpin>;
