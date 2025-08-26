use rand_chacha::ChaCha12Rng;

use crate::{
    core::{backends::BlendBackend, BlendService},
    message::ServiceMessage,
};

pub trait ServiceComponents<RuntimeServiceId> {
    type NetworkAdapter;
    type BlendBackend;
    type NodeId;
    type Rng;
    type Message;
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
    type Message = ServiceMessage<Network::BroadcastSettings>;
}

pub type NetworkBackendOfService<Service, RuntimeServiceId> = <<Service as ServiceComponents<
    RuntimeServiceId,
>>::NetworkAdapter as crate::core::network::NetworkAdapter<RuntimeServiceId>>::Backend;
pub type NetworkBroadcastSettingsOfService<Service, RuntimeServiceId> = <<Service as ServiceComponents<
    RuntimeServiceId,
>>::NetworkAdapter as crate::core::network::NetworkAdapter<RuntimeServiceId>>::BroadcastSettings;

pub trait MessageComponents {
    type Payload;
    type BroadcastSettings;

    fn payload(&self) -> &Self::Payload;
    fn broadcast_settings(&self) -> &Self::BroadcastSettings;
}

impl<BroadcastSettings> MessageComponents for ServiceMessage<BroadcastSettings> {
    type Payload = Vec<u8>;
    type BroadcastSettings = BroadcastSettings;

    fn payload(&self) -> &Self::Payload {
        let Self::Blend(network_message) = self;
        &network_message.message
    }

    fn broadcast_settings(&self) -> &Self::BroadcastSettings {
        let Self::Blend(network_message) = self;
        &network_message.broadcast_settings
    }
}
