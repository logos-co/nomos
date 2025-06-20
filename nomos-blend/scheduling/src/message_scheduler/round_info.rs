use crate::message::{CoverMessage, OutboundMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundInfo {
    pub processed_messages: Vec<OutboundMessage>,
    pub cover_message: Option<CoverMessage>,
}
