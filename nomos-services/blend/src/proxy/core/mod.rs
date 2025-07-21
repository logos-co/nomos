#[cfg(feature = "libp2p")]
pub mod libp2p;

use overwatch::services::relay::OutboundRelay;

use crate::{
    core::{backends::BlendBackend, network::NetworkAdapter},
    message::ServiceMessage,
};

const LOG_TARGET: &str = "blend::service::proxy::core";

#[async_trait::async_trait]
pub trait Adapter<RuntimeServiceId> {
    type Backend: BlendBackend<Self::NodeId, RuntimeServiceId> + 'static;
    type Network: NetworkAdapter<RuntimeServiceId> + 'static;
    type NodeId;

    fn new(
        relay: OutboundRelay<
            ServiceMessage<<Self::Network as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings>,
        >,
    ) -> Self;

    async fn relay(
        &self,
        service_message: ServiceMessage<
            <Self::Network as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings,
        >,
    );
}
