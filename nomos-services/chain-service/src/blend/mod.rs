pub mod adapters;

use nomos_blend_service::{backends::BlendBackend, network::NetworkAdapter, BlendService};
use nomos_core::block::Block;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use rand_chacha::ChaCha12Rng;
use serde::{de::DeserializeOwned, Serialize};

#[async_trait::async_trait]
pub trait BlendAdapter<RuntimeServiceId> {
    type Settings: Clone + 'static;
    type Backend: BlendBackend<Self::NodeId, ChaCha12Rng, RuntimeServiceId> + 'static;
    type Network: NetworkAdapter<RuntimeServiceId> + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + 'static;
    type NodeId;

    async fn new(
        settings: Self::Settings,
        blend_relay: OutboundRelay<
            <BlendService<Self::Backend, Self::NodeId, Self::Network, RuntimeServiceId> as ServiceData>::Message
        >,
    ) -> Self;
    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>);
}
