pub mod adapters;

use nomos_blend_service::{backends::BlendBackend, network::NetworkAdapter, BlendService};
use nomos_core::block::Block;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Serialize};

#[async_trait::async_trait]
pub trait BlendAdapter<RuntimeServiceId> {
    type Settings: Clone + 'static;
    type Backend: BlendBackend<RuntimeServiceId> + 'static;
    type Network: NetworkAdapter<RuntimeServiceId> + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + 'static;
    async fn new(
        settings: Self::Settings,
        blend_relay: OutboundRelay<
            <BlendService<Self::Backend, Self::Network, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;
    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>);
}
