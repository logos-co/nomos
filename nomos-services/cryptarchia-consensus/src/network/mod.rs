pub mod adapters;

use std::hash::Hash;

use cryptarchia_engine::Slot;
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use futures::Stream;
use nomos_core::{block::Block, header::HeaderId};
use nomos_network::{
    backends::{libp2p::SyncRequestKind, NetworkBackend},
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

pub struct SyncRequest {
    pub kind: SyncRequestKind,
    pub reply_channel: Sender<BehaviourSyncReply>,
}

type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + 'static;
    type Settings: Clone + 'static;
    type Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    type BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    async fn request_sync(&self, slot: u64) -> Result<UnboundedReceiver<Vec<u8>>, DynError>;

    async fn blocks_stream(
        &self,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError>;
    async fn fetch_blocks_from_slot(
        &self,
        start_slot: Slot,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError>;
    async fn fetch_chain_backward(
        &self,
        tip: HeaderId,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError>;

    async fn sync_requests_stream(&self) -> Result<BoxedStream<SyncRequest>, DynError>;
}
