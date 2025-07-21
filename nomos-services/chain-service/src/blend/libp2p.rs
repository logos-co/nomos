use std::marker::PhantomData;

use nomos_blend_service::{
    core::network::NetworkAdapter,
    message::ServiceMessage,
    proxy::{self, BlendProxyService},
};
use nomos_core::{block::Block, wire};
use nomos_network::backends::libp2p::PeerId;
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{blend::BlendAdapter, messages::NetworkMessage};

#[derive(Clone)]
pub struct LibP2pAdapter<Tx, BlobCert, RuntimeServiceId>
where
    Tx: Clone + Eq,
    BlobCert: Clone + Eq,
    RuntimeServiceId: 'static,
{
    settings: LibP2pAdapterSettings<BroadcastSettings<RuntimeServiceId>>,
    relay: OutboundRelay<<Service<RuntimeServiceId> as ServiceData>::Message>,
    _phantom: PhantomData<(Tx, BlobCert)>,
}

type Service<RuntimeServiceId> = BlendProxyService<
    proxy::core::libp2p::Libp2pAdapter<RuntimeServiceId>,
    proxy::edge::libp2p::Libp2pAdapter<RuntimeServiceId>,
    RuntimeServiceId,
>;
type BroadcastSettings<RuntimeServiceId> =
    <<proxy::core::libp2p::Libp2pAdapter<RuntimeServiceId> as proxy::core::Adapter<
        RuntimeServiceId,
    >>::Network as NetworkAdapter<RuntimeServiceId>>::BroadcastSettings;

#[async_trait::async_trait]
impl<Tx, BlobCert, RuntimeServiceId> BlendAdapter<RuntimeServiceId>
    for LibP2pAdapter<Tx, BlobCert, RuntimeServiceId>
where
    Tx: Serialize + DeserializeOwned + Clone + Eq + Send + Sync + 'static,
    BlobCert: Serialize + DeserializeOwned + Clone + Eq + Send + Sync + 'static,
    RuntimeServiceId: 'static,
{
    type Settings = LibP2pAdapterSettings<BroadcastSettings<RuntimeServiceId>>;
    type CoreAdapter = proxy::core::libp2p::Libp2pAdapter<RuntimeServiceId>;
    type EdgeAdapter = proxy::edge::libp2p::Libp2pAdapter<RuntimeServiceId>;
    type Tx = Tx;
    type BlobCertificate = BlobCert;
    type NodeId = PeerId;

    async fn new(
        settings: Self::Settings,
        relay: OutboundRelay<
            <BlendProxyService<Self::CoreAdapter, Self::EdgeAdapter, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        // this wait seems to be helpful in some cases since we give the time
        // to the network to establish connections before we start sending messages
        // TODO: Remove this once we have the status system to await for service
        // readiness
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        Self {
            settings,
            relay,
            _phantom: PhantomData,
        }
    }

    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>) {
        if let Err((e, msg)) = self
            .relay
            .send(ServiceMessage::Blend(
                nomos_blend_service::message::NetworkMessage {
                    message: wire::serialize(&NetworkMessage::Block(block)).unwrap(),
                    broadcast_settings: self.settings.broadcast_settings.clone(),
                },
            ))
            .await
        {
            tracing::error!("error sending message to blend network: {e}: {msg:?}",);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibP2pAdapterSettings<BroadcastSettings> {
    pub broadcast_settings: BroadcastSettings,
}
