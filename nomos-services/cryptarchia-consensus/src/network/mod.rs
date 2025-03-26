pub mod adapters;

use std::hash::Hash;

use futures::Stream;
use nomos_core::block::Block;
use nomos_network::{backends::NetworkBackend, NetworkService};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct SyncRequest {
    pub slot: u64,
    pub reply_channel: UnboundedSender<Vec<u8>>,
}

type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter {
    type Backend: NetworkBackend + 'static;
    type Settings: Clone + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<<NetworkService<Self::Backend> as ServiceData>::Message>,
    ) -> Self;

    async fn request_sync(&self, slot: u64) -> Result<UnboundedReceiver<Vec<u8>>, DynError>;

    async fn blocks_stream(
        &self,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError>;

    async fn sync_requests_stream(&self) -> Result<BoxedStream<SyncRequest>, DynError>;
}
