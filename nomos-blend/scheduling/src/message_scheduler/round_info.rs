use crate::message::{BlendOutgoingMessage, CoverMessage};

/// Information can the message scheduler can yield when being polled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundInfo {
    /// The list of *unshuffled* messages to be released.
    pub processed_messages: Vec<BlendOutgoingMessage>,
    /// The cover message to generate, if present.
    pub cover_message: Option<CoverMessage>,
}
