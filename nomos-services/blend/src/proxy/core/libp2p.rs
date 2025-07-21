use libp2p::PeerId;
use overwatch::services::{relay::OutboundRelay, ServiceData};

use crate::{
    core::{
        backends::libp2p::Libp2pBlendBackend,
        network::{libp2p::Libp2pAdapter as Libp2pNetworkAdapter, NetworkAdapter},
        BlendService,
    },
    message::ServiceMessage,
    proxy::core::Adapter,
};

const LOG_TARGET: &str = "blend::service::proxy::core::libp2p";

pub struct Libp2pAdapter<RuntimeServiceId> {
    relay: OutboundRelay<<BlendServiceType<RuntimeServiceId> as ServiceData>::Message>,
}

type BlendServiceType<RuntimeServiceId> = BlendService<
    Libp2pBlendBackend,
    PeerId,
    Libp2pNetworkAdapter<RuntimeServiceId>,
    RuntimeServiceId,
>;

#[async_trait::async_trait]
impl<RuntimeServiceId> Adapter<RuntimeServiceId> for Libp2pAdapter<RuntimeServiceId>
where
    RuntimeServiceId: 'static,
{
    type Backend = Libp2pBlendBackend;
    type Network = Libp2pNetworkAdapter<RuntimeServiceId>;
    type NodeId = PeerId;

    fn new(
        relay: OutboundRelay<
            ServiceMessage<<Self::Network as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings>,
        >,
    ) -> Self {
        Self { relay }
    }

    async fn relay(
        &self,
        service_message: ServiceMessage<
            <Self::Network as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        >,
    ) {
        if let Err((e, _)) = self.relay.send(service_message).await {
            tracing::error!(target: LOG_TARGET, "error relaying message to core service: {e}");
        }
    }
}
