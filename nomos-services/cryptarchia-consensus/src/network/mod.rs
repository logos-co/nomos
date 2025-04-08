pub mod adapters;

use std::hash::Hash;

use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use futures::Stream;
use nomos_network::{
    backends::{libp2p::SyncRequestKind, NetworkBackend},
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub kind: SyncRequestKind,
    pub reply_channel: Sender<BehaviourSyncReply>,
}

type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Sync + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + 'static;
    type Settings: Clone + 'static;
    type Block: Serialize + DeserializeOwned + Clone + Eq + Hash + 'static;
    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;

    async fn blocks_stream(&self) -> Result<BoxedStream<Self::Block>, DynError>;

    async fn sync_requests_stream(&self) -> Result<BoxedStream<SyncRequest>, DynError>;
}
