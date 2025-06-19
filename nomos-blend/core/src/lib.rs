pub mod cover_traffic;
pub mod membership;
pub mod message_blend;
pub mod persistent_transmission;

pub enum BlendOutgoingMessage {
    FullyUnwrapped(Vec<u8>),
    Outbound(Vec<u8>),
}

impl From<BlendOutgoingMessage> for Vec<u8> {
    fn from(value: BlendOutgoingMessage) -> Self {
        match value {
            BlendOutgoingMessage::FullyUnwrapped(v) | BlendOutgoingMessage::Outbound(v) => v,
        }
    }
}
