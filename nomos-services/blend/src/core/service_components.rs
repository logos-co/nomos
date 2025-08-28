use rand_chacha::ChaCha12Rng;

use crate::{
    core::{backends::BlendBackend, BlendService},
    message::ServiceMessage,
};

/// Helper trait to help the Blend proxy service rely on the concrete types of
/// the core Blend service without having to specify all the generics the core
/// service expects.
pub trait ServiceComponents<RuntimeServiceId> {
    type NetworkAdapter;
    type BlendBackend;
    type NodeId;
    type Rng;
}

impl<Backend, NodeId, Network, RuntimeServiceId> ServiceComponents<RuntimeServiceId>
    for BlendService<Backend, NodeId, Network, RuntimeServiceId>
where
    Backend: BlendBackend<NodeId, ChaCha12Rng, RuntimeServiceId>,
    Network: crate::core::network::NetworkAdapter<RuntimeServiceId>,
{
    type NetworkAdapter = Network;
    type BlendBackend = Backend;
    type NodeId = NodeId;
    type Rng = ChaCha12Rng;
}

pub type NetworkBackendOfService<Service, RuntimeServiceId> = <<Service as ServiceComponents<
    RuntimeServiceId,
>>::NetworkAdapter as crate::core::network::NetworkAdapter<RuntimeServiceId>>::Backend;

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
