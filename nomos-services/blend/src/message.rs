use futures::Stream;
use nomos_blend_message::crypto::proofs::quota::inputs::prove::private::ProofOfLeadershipQuotaInputs;
use nomos_blend_scheduling::EncapsulatedMessage;
use nomos_core::crypto::ZkHash;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct PolEpochInfo {
    pub epoch_nonce: ZkHash,
    pub poq_private_inputs: ProofOfLeadershipQuotaInputs,
}

/// A message that is handled by [`BlendService`].
pub enum ServiceMessage<BroadcastSettings> {
    /// To send a message to the blend network and eventually broadcast it to
    /// the [`NetworkService`].
    Blend(NetworkMessage<BroadcastSettings>),
    /// Notify Blend service about a new `PoL` epoch, with information about any
    /// one winning slot.
    PolEpochStream(Box<dyn Stream<Item = PolEpochInfo> + Send + Sync + Unpin>),
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
