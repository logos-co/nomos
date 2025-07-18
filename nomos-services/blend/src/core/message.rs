use nomos_blend_scheduling::{BlendOutgoingMessage, DataMessage, EncapsulatedMessage};
use nomos_core::wire;
use serde::Serialize;

use crate::message::NetworkMessage;

#[derive(Debug)]
pub enum ProcessedMessage<BroadcastSettings> {
    Network(NetworkMessage<BroadcastSettings>),
    Encapsulated(EncapsulatedMessage),
}

impl<BroadcastSettings> From<NetworkMessage<BroadcastSettings>>
    for ProcessedMessage<BroadcastSettings>
{
    fn from(value: NetworkMessage<BroadcastSettings>) -> Self {
        Self::Network(value)
    }
}

impl<BroadcastSettings> From<EncapsulatedMessage> for ProcessedMessage<BroadcastSettings> {
    fn from(value: EncapsulatedMessage) -> Self {
        Self::Encapsulated(value)
    }
}

impl<BroadcastSettings> TryFrom<ProcessedMessage<BroadcastSettings>> for BlendOutgoingMessage
where
    BroadcastSettings: Serialize,
{
    type Error = wire::Error;

    fn try_from(value: ProcessedMessage<BroadcastSettings>) -> Result<Self, Self::Error> {
        match value {
            ProcessedMessage::Encapsulated(encapsulated) => {
                Ok(Self::EncapsulatedMessage(encapsulated))
            }
            ProcessedMessage::Network(unserialized_network_message) => Ok(Self::DataMessage(
                DataMessage::from(wire::serialize(&unserialized_network_message)?),
            )),
        }
    }
}
