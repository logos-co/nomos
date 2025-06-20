#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlendOutgoingMessage {
    FullyUnwrapped(FullyUnwrappedMessage),
    Outbound(OutboundMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullyUnwrappedMessage(Vec<u8>);

impl From<Vec<u8>> for FullyUnwrappedMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<FullyUnwrappedMessage> for Vec<u8> {
    fn from(value: FullyUnwrappedMessage) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for FullyUnwrappedMessage {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMessage(Vec<u8>);

impl From<Vec<u8>> for OutboundMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<OutboundMessage> for Vec<u8> {
    fn from(value: OutboundMessage) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for OutboundMessage {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<BlendOutgoingMessage> for Vec<u8> {
    fn from(value: BlendOutgoingMessage) -> Self {
        match value {
            BlendOutgoingMessage::FullyUnwrapped(FullyUnwrappedMessage(v))
            | BlendOutgoingMessage::Outbound(OutboundMessage(v)) => v,
        }
    }
}

pub struct CoverMessage(Vec<u8>);

impl From<CoverMessage> for Vec<u8> {
    fn from(value: CoverMessage) -> Self {
        value.0
    }
}

impl From<Vec<u8>> for CoverMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
