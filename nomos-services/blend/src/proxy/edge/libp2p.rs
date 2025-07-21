use libp2p::PeerId;
use overwatch::services::{relay::OutboundRelay, ServiceData};

use crate::{
    core::network::{libp2p::Libp2pAdapter as Libp2pNetworkAdapter, NetworkAdapter},
    edge::{backends::libp2p::Libp2pBlendBackend, BlendService},
    message::ServiceMessage,
    proxy::edge::{Adapter, LOG_TARGET},
};

pub struct Libp2pAdapter<RuntimeServiceId> {
    relay: OutboundRelay<<BlendServiceType<RuntimeServiceId> as ServiceData>::Message>,
}

type BlendServiceType<RuntimeServiceId> = BlendService<
    Libp2pBlendBackend,
    PeerId,
    <Libp2pNetworkAdapter<RuntimeServiceId> as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
    RuntimeServiceId,
>;

#[async_trait::async_trait]
impl<RuntimeServiceId> Adapter<RuntimeServiceId> for Libp2pAdapter<RuntimeServiceId> {
    type Backend = Libp2pBlendBackend;
    type BroadcastSettings = <Libp2pNetworkAdapter<RuntimeServiceId> as NetworkAdapter<
        RuntimeServiceId,
    >>::BroadcastSettings;
    type NodeId = PeerId;

    fn new(relay: OutboundRelay<ServiceMessage<Self::BroadcastSettings>>) -> Self {
        Self { relay }
    }

    async fn relay(&self, service_message: ServiceMessage<Self::BroadcastSettings>) {
        if let Err((e, _)) = self.relay.send(service_message).await {
            tracing::error!(target: LOG_TARGET, "error relaying message to edge service: {e}");
        }
    }
}
