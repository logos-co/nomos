use std::{hash::Hash, marker::PhantomData};

use nomos_blend_service::{
    backends::libp2p::Libp2pBlendBackend, network::NetworkAdapter, BlendService, ServiceMessage,
};
use nomos_core::{block::Block, wire};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{blend::BlendAdapter, messages::NetworkMessage};

#[derive(Clone)]
pub struct LibP2pAdapter<Network, Tx, BlobCert, RuntimeServiceId>
where
    Network: NetworkAdapter<RuntimeServiceId>,
    Network::BroadcastSettings: Clone,
    Tx: Clone + Eq + Hash,
    BlobCert: Clone + Eq + Hash,
{
    settings: LibP2pAdapterSettings<Network::BroadcastSettings>,
    blend_relay: OutboundRelay<
        <BlendService<Libp2pBlendBackend, Network, RuntimeServiceId> as ServiceData>::Message,
    >,
    _tx: PhantomData<Tx>,
    _blob_cert: PhantomData<BlobCert>,
}

#[async_trait::async_trait]
impl<Network, Tx, BlobCert, RuntimeServiceId> BlendAdapter<RuntimeServiceId>
    for LibP2pAdapter<Network, Tx, BlobCert, RuntimeServiceId>
where
    Network: NetworkAdapter<RuntimeServiceId> + 'static,
    Network::BroadcastSettings: Clone,
    Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
    BlobCert: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
{
    type Settings = LibP2pAdapterSettings<Network::BroadcastSettings>;
    type Backend = Libp2pBlendBackend;
    type Network = Network;
    type Tx = Tx;
    type BlobCertificate = BlobCert;

    async fn new(
        settings: Self::Settings,
        blend_relay: OutboundRelay<
            <BlendService<Self::Backend, Self::Network, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        // this wait seems to be helpful in some cases since we give the time
        // to the network to establish connections before we start sending messages
        // TODO: Remove this once we have the status system to await for service
        // readiness
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        Self {
            settings,
            blend_relay,
            _tx: PhantomData,
            _blob_cert: PhantomData,
        }
    }

    async fn blend(&self, block: Block<Self::Tx, Self::BlobCertificate>) {
        if let Err((e, msg)) = self
            .blend_relay
            .send(ServiceMessage::Blend(nomos_blend_service::NetworkMessage {
                message: wire::serialize(&NetworkMessage::Block(block)).unwrap(),
                broadcast_settings: self.settings.broadcast_settings.clone(),
            }))
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
