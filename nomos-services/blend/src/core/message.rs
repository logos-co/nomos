use nomos_blend_scheduling::EncapsulatedMessage;

use crate::message::NetworkMessage;

#[derive(Debug)]
pub enum ProcessedMessage<BroadcastSettings> {
    Network(NetworkMessage<BroadcastSettings>),
    Encapsulated(Box<EncapsulatedMessage>),
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
        Self::Encapsulated(Box::new(value))
    }
}
