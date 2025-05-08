pub mod adapters;

use std::hash::Hash;

use nomos_blend_service::{BlendService, backends::BlendBackend, network::NetworkAdapter};
use nomos_core::block::Block;
use overwatch::services::{ServiceData, relay::OutboundRelay};
use serde::{Serialize, de::DeserializeOwned};

#[async_trait::async_trait]
pub trait BlendAdapter<RuntimeServiceId> {
    type Settings: Clone + 'static;
    type Backend: BlendBackend<RuntimeServiceId> + 'static;
    type Network: NetworkAdapter<RuntimeServiceId> + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    async fn new(
        settings: Self::Settings,
        blend_relay: OutboundRelay<
            <BlendService<Self::Backend, Self::Network, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;
    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>);
}
