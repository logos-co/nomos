use std::{collections::HashSet, fmt::Debug};

use futures::stream::BoxStream;
use nomos_libp2p::{
    cryptarchia_sync,
    cryptarchia_sync::{ChainSyncError, HeaderId},
    libp2p::bytes::Bytes,
};
use tokio::sync::{mpsc::Sender, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use crate::backends::NetworkBackend;

type BlocksStream = BoxStream<'static, Result<Bytes, ChainSyncError>>;

#[derive(Debug)]
pub enum NetworkMsg<Payload, PubSubEvent, ChainSyncEvent> {
    Process(Payload),
    SubscribeToPubSub {
        sender: oneshot::Sender<BroadcastStream<PubSubEvent>>,
    },
    SubscribeToChainSync {
        sender: oneshot::Sender<BroadcastStream<ChainSyncEvent>>,
    },
}

pub type BackendNetworkMsg<Backend, RuntimeServiceId> = NetworkMsg<
    <Backend as NetworkBackend<RuntimeServiceId>>::Message,
    <Backend as NetworkBackend<RuntimeServiceId>>::PubSubEvent,
    <Backend as NetworkBackend<RuntimeServiceId>>::ChainSyncEvent,
>;

#[derive(Debug, Clone)]
pub enum ChainSyncEvent {
    ProvideBlocksRequest {
        /// Return blocks up to `target_block`.
        target_block: HeaderId,
        /// The local canonical chain latest block.
        local_tip: HeaderId,
        /// The latest immutable block.
        latest_immutable_block: HeaderId,
        /// The list of additional blocks that the requester has.
        additional_blocks: HashSet<HeaderId>,
        /// Channel to send blocks to the service.
        reply_sender: Sender<BlocksStream>,
    },
    ProvideTipRequest {
        /// Channel to send the latest tip to the service.
        reply_sender: Sender<HeaderId>,
    },
}

// Convert libp2p specific type to a common type.
impl From<cryptarchia_sync::Event> for ChainSyncEvent {
    fn from(event: cryptarchia_sync::Event) -> Self {
        match event {
            cryptarchia_sync::Event::ProvideBlocksRequest {
                target_block,
                local_tip,
                latest_immutable_block,
                additional_blocks,
                reply_sender,
            } => Self::ProvideBlocksRequest {
                target_block,
                local_tip,
                latest_immutable_block,
                additional_blocks,
                reply_sender,
            },
            cryptarchia_sync::Event::ProvideTipsRequest { reply_sender } => {
                Self::ProvideTipRequest { reply_sender }
            }
        }
    }
}
