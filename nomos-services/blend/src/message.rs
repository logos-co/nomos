use nomos_blend_scheduling::{BlendOutgoingMessage, DataMessage, EncapsulatedMessage};
use nomos_core::wire;
use serde::{Deserialize, Serialize};

/// A message that is handled by [`BlendService`].
#[derive(Debug)]
pub enum ServiceMessage<BroadcastSettings> {
    /// To send a message to the blend network and eventually broadcast it to
    /// the [`NetworkService`].
    Blend(NetworkMessage<BroadcastSettings>),
}

/// A message that is sent to the blend network.
///
/// To eventually broadcast the message to the network service,
/// [`BroadcastSettings`] must be included in the [`NetworkMessage`].
/// [`BroadcastSettings`] is a generic type defined by [`NetworkAdapter`].
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkMessage<BroadcastSettings> {
    pub message: Vec<u8>,
    pub broadcast_settings: BroadcastSettings,
}

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
