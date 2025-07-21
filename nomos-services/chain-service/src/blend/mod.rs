#[cfg(feature = "libp2p")]
pub mod libp2p;

use nomos_blend_service::proxy::{self, BlendProxyService};
use nomos_core::block::Block;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Serialize};

#[async_trait::async_trait]
pub trait BlendAdapter<RuntimeServiceId> {
    type Settings: Clone + 'static;
    type CoreAdapter: proxy::core::Adapter<RuntimeServiceId>;
    type EdgeAdapter: proxy::edge::Adapter<RuntimeServiceId>;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + 'static;
    type NodeId;

    async fn new(
        settings: Self::Settings,
        blend_relay: OutboundRelay<
            <BlendProxyService<Self::CoreAdapter, Self::EdgeAdapter, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;
    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>);
}
