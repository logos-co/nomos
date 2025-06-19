pub enum BlendOutgoingMessage {
    FullyUnwrapped(FullyUnwrappedMessage),
    Outbound(OutboundMessage),
}

pub struct FullyUnwrappedMessage(Vec<u8>);

pub struct OutboundMessage(Vec<u8>);

pub struct CoverMessage(Vec<u8>);

impl From<BlendOutgoingMessage> for Vec<u8> {
    fn from(value: BlendOutgoingMessage) -> Self {
        match value {
            BlendOutgoingMessage::FullyUnwrapped(FullyUnwrappedMessage(v))
            | BlendOutgoingMessage::Outbound(OutboundMessage(v)) => v,
        }
    }
}

impl From<CoverMessage> for Vec<u8> {
    fn from(value: CoverMessage) -> Self {
        value.0
    }
}
