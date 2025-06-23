use core::fmt;
use std::fmt::{Display, Formatter};

use crate::message::{CoverMessage, OutboundMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Round(u128);

impl Round {
    pub fn inner(&self) -> u128 {
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
    /// The list of *unshuffled* messages to be released.
    pub processed_messages: Vec<OutboundMessage>,
    /// The cover message to generate, if present.
    pub cover_message: Option<CoverMessage>,
}
