#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlendOutgoingMessage {
    FullyUnwrapped(FullyUnwrappedMessage),
    Outbound(OutboundMessage),
}

impl AsRef<[u8]> for BlendOutgoingMessage {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::FullyUnwrapped(msg) => msg.as_ref(),
            Self::Outbound(msg) => msg.as_ref(),
        }
    }
}

impl From<FullyUnwrappedMessage> for BlendOutgoingMessage {
    fn from(value: FullyUnwrappedMessage) -> Self {
        Self::FullyUnwrapped(value)
    }
}

impl From<OutboundMessage> for BlendOutgoingMessage {
    fn from(value: OutboundMessage) -> Self {
        Self::Outbound(value)
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
