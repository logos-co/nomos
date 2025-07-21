#[cfg(feature = "libp2p")]
pub mod libp2p;

use overwatch::services::relay::OutboundRelay;

use crate::{edge::backends::BlendEdgeBackend, message::ServiceMessage};

#[async_trait::async_trait]
pub trait Adapter<RuntimeServiceId> {
    type Backend: BlendEdgeBackend<Self::NodeId, RuntimeServiceId> + 'static;
    type BroadcastSettings: 'static;
    type NodeId: Clone;

    fn new(relay: OutboundRelay<ServiceMessage<Self::BroadcastSettings>>) -> Self;

    async fn relay(&self, service_message: ServiceMessage<Self::BroadcastSettings>);
}
