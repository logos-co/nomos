pub mod membership;
pub mod message_blend;
pub mod message_scheduler;
mod serde;

mod cover_traffic;
mod release_delayer;

pub use self::message_scheduler::UninitializedMessageScheduler;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlendOutgoingMessage {
    CoverMessage(CoverMessage),
    DataMessage(DataMessage),
    EncapsulatedMessage(EncapsulatedMessage),
}

impl From<BlendOutgoingMessage> for Vec<u8> {
    fn from(value: BlendOutgoingMessage) -> Self {
        match value {
            BlendOutgoingMessage::CoverMessage(m) => m.into(),
            BlendOutgoingMessage::DataMessage(m) => m.into(),
            BlendOutgoingMessage::EncapsulatedMessage(m) => m.into(),
        }
    }
}

impl AsRef<[u8]> for BlendOutgoingMessage {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::CoverMessage(m) => m.as_ref(),
            Self::DataMessage(m) => m.as_ref(),
            Self::EncapsulatedMessage(m) => m.as_ref(),
        }
    }
}

impl From<CoverMessage> for BlendOutgoingMessage {
    fn from(value: CoverMessage) -> Self {
        Self::CoverMessage(value)
    }
}

impl From<DataMessage> for BlendOutgoingMessage {
    fn from(value: DataMessage) -> Self {
        Self::DataMessage(value)
    }
}

impl From<EncapsulatedMessage> for BlendOutgoingMessage {
    fn from(value: EncapsulatedMessage) -> Self {
        Self::EncapsulatedMessage(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoverMessage(Vec<u8>);

impl From<Vec<u8>> for CoverMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<CoverMessage> for Vec<u8> {
    fn from(value: CoverMessage) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for CoverMessage {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataMessage(Vec<u8>);

impl From<Vec<u8>> for DataMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<DataMessage> for Vec<u8> {
    fn from(value: DataMessage) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for DataMessage {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EncapsulatedMessage(Vec<u8>);

impl From<Vec<u8>> for EncapsulatedMessage {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<EncapsulatedMessage> for Vec<u8> {
    fn from(value: EncapsulatedMessage) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for EncapsulatedMessage {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
