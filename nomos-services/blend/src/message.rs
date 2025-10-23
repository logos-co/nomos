use nomos_blend_scheduling::EncapsulatedMessage;
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

pub trait MessageComponents {
    type Payload;
    type BroadcastSettings;

    fn into_components(self) -> (Self::Payload, Self::BroadcastSettings);
}

impl<BroadcastSettings> MessageComponents for ServiceMessage<BroadcastSettings> {
    type Payload = Vec<u8>;
    type BroadcastSettings = BroadcastSettings;

    fn into_components(self) -> (Self::Payload, Self::BroadcastSettings) {
        let Self::Blend(network_message) = self;
        (network_message.message, network_message.broadcast_settings)
    }
}
