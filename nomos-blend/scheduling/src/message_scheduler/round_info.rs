use crate::message::{CoverMessage, OutboundMessage};

pub struct RoundInfo {
    pub processed_messages: Vec<OutboundMessage>,
    pub cover_message: Option<CoverMessage>,
}
