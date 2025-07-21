#[cfg(feature = "libp2p")]
pub mod libp2p;

use overwatch::services::relay::OutboundRelay;

use crate::{edge::backends::BlendBackend, message::ServiceMessage};

const LOG_TARGET: &str = "blend::service::proxy::edge";

#[async_trait::async_trait]
pub trait Adapter<RuntimeServiceId> {
    type Backend: BlendBackend<Self::NodeId, RuntimeServiceId> + 'static;
    type BroadcastSettings: 'static;
    type NodeId: Clone;

    fn new(relay: OutboundRelay<ServiceMessage<Self::BroadcastSettings>>) -> Self;

    async fn relay(&self, service_message: ServiceMessage<Self::BroadcastSettings>);
}
