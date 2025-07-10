use std::{collections::HashSet, fmt::Debug};

use bytes::Bytes;
use futures::stream::BoxStream;
use nomos_core::header::HeaderId;
use nomos_libp2p::cryptarchia_sync::GetTipResponse;
use overwatch::DynError;
use tokio::sync::{mpsc::Sender, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use crate::backends::NetworkBackend;

type BlocksStream = BoxStream<'static, Result<Bytes, DynError>>;

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
        reply_sender: Sender<GetTipResponse>,
    },
}
