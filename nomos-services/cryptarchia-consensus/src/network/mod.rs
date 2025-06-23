pub mod adapters;

use std::hash::Hash;

use futures::Stream;
use nomos_core::{block::Block, header::HeaderId};
use nomos_network::{backends::NetworkBackend, message::ChainSyncEvent, NetworkService};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Serialize};

type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + 'static;
    type Settings: Clone + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    type PeerId;
    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;
    async fn blocks_stream(
        &self,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError>;

    async fn chainsync_events_stream(&self) -> Result<BoxedStream<ChainSyncEvent>, DynError>;

    async fn request_tip(&self, peer: Self::PeerId) -> Result<HeaderId, DynError>;
}
