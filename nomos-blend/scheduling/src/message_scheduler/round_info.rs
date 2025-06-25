use core::{
    fmt,
    fmt::{Display, Formatter},
};

use futures::Stream;

use crate::{DataMessage, EncapsulatedMessage};

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

/// Information can the message scheduler can yield when being polled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundInfo {
    /// The list of messages to be released.
    pub processed_messages: Vec<ProcessedMessage>,
    /// Flag indicating (if `Some`) whether a new cover message should be
    /// generated during this release round.
    pub cover_message_generation_flag: Option<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessedMessage {
    Data(DataMessage),
    Encapsulated(EncapsulatedMessage),
}

impl From<DataMessage> for ProcessedMessage {
    fn from(value: DataMessage) -> Self {
        Self::Data(value)
    }
}

impl From<EncapsulatedMessage> for ProcessedMessage {
    fn from(value: EncapsulatedMessage) -> Self {
        Self::Encapsulated(value)
    }
}

pub type RoundClock = Box<dyn Stream<Item = Round> + Unpin>;
