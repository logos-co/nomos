pub mod adapters;

use std::collections::HashSet;

use cryptarchia_sync::GetTipResponse;
use futures::Stream;
use nomos_core::header::HeaderId;
use nomos_network::{backends::NetworkBackend, message::ChainSyncEvent, NetworkService};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};

pub(crate) type BoxedStream<T> = Box<dyn Stream<Item = T> + Send + Unpin>;

#[async_trait::async_trait]
pub trait NetworkAdapter<RuntimeServiceId> {
    type Backend: NetworkBackend<RuntimeServiceId> + 'static;
    type Settings: Clone + 'static;
    type PeerId;
    type Block;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self;
    async fn blocks_stream(&self) -> Result<BoxedStream<Self::Block>, DynError>;

    async fn chainsync_events_stream(&self) -> Result<BoxedStream<ChainSyncEvent>, DynError>;

    async fn request_tip(&self, peer: Self::PeerId) -> Result<GetTipResponse, DynError>;

    async fn request_blocks_from_peer(
        &self,
        peer: Self::PeerId,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: HashSet<HeaderId>,
    ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError>;
}
