use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use cryptarchia_engine::Slot;
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use nomos_core::{block::AbstractBlock, header::HeaderId, wire};
use nomos_network::backends::libp2p::SyncRequestKind;
use nomos_storage::{ScanResult, StorageMsg, StorageService, backends::StorageBackend};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    runtime::Handle,
    sync::{mpsc::Sender, oneshot},
    time::{Duration, sleep},
};
use tracing::info;

use crate::{network::SyncRequest, storage::BLOCK_INDEX_PREFIX};

const MAX_CONCURRENT_REQUESTS: usize = 1;
const BLOCK_SEND_DELAY_MS: u64 = 50;

type StorageRelay<Storage, RuntimeServiceId> =
    OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>;

pub struct SyncBlocksProvider<
    Storage: StorageBackend + Send + Sync + 'static,
    Block,
    RuntimeServiceId,
> {
    storage_relay:
        OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>,
    runtime: Handle,
    in_progress_requests: Arc<AtomicUsize>,
    phantom_data: PhantomData<Block>,
}

impl<Storage, Block, RuntimeServiceId> SyncBlocksProvider<Storage, Block, RuntimeServiceId>
where
    Storage: StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(
        storage_relay: OutboundRelay<
            <StorageService<Storage, RuntimeServiceId> as ServiceData>::Message,
        >,
        runtime: Handle,
    ) -> Self {
        Self {
            storage_relay,
            runtime,
            in_progress_requests: Arc::new(AtomicUsize::new(0)),
            phantom_data: PhantomData,
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Seems strange because it does not have a lot of logic."
    )]
    pub fn process_sync_request(&self, request: SyncRequest) {
        if self.in_progress_requests.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_REQUESTS {
            tracing::warn!("Max concurrent sync requests reached");
            self.in_progress_requests.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        match request.kind {
            SyncRequestKind::ForwardChain(slot) => {
                info!("Syncing from slot {:?}", slot);
                self.spawn_fetch_blocks_from_slot_forwards(slot, request.reply_channel);
            }
            SyncRequestKind::BackwardChain { start_block, .. } => {
                info!("Syncing from header {:?}", start_block);
                self.spawn_fetch_blocks_from_header_backwards(start_block, request.reply_channel);
            }
            SyncRequestKind::Tip => {
                tracing::warn!("Unsupported sync request kind");
                self.in_progress_requests.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    fn spawn_fetch_blocks_from_slot_forwards(
        &self,
        slot: Slot,
        reply_channel: Sender<BehaviourSyncReply>,
    ) {
        let storage_relay = self.storage_relay.clone();
        let in_progress_requests = Arc::clone(&self.in_progress_requests);
        self.runtime.spawn(async move {
            match Self::get_blocks_from_slot(&storage_relay, slot).await {
                Ok(mut block_receiver) => {
                    while let Some(result) = block_receiver.recv().await {
                        match result {
                            Ok((_key, value)) => {
                                let block: Block =
                                    wire::deserialize(&value).expect("Deserialization failed");
                                if Self::send_block(&block, &reply_channel).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to process block: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load prefix: {}", e);
                }
            }
            in_progress_requests.fetch_sub(1, Ordering::SeqCst);
        });
    }

    fn spawn_fetch_blocks_from_header_backwards(
        &self,
        header_id: HeaderId,
        reply_channel: Sender<BehaviourSyncReply>,
    ) {
        let storage_relay = self.storage_relay.clone();
        let in_progress_requests = Arc::clone(&self.in_progress_requests);
        self.runtime.spawn(async move {
            let mut current_header_id = header_id;
            loop {
                info!("Loading block from header {:?}", current_header_id);
                let key: [u8; 32] = current_header_id.into();
                let bytes = Bytes::copy_from_slice(&key);
                match Self::load_block(&storage_relay, bytes).await {
                    Ok(Some(block)) => {
                        let block: Block =
                            wire::deserialize(&block).expect("Deserialization failed");
                        if Self::send_block(&block, &reply_channel).await.is_err() {
                            break;
                        }
                        current_header_id = block.parent();
                        if current_header_id == [0; 32].into() {
                            tracing::debug!("Reached genesis block");
                            break;
                        }
                    }
                    Ok(None) => {
                        tracing::debug!("No more blocks found for header {:?}", current_header_id);
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Failed to load block: {}", e);
                        break;
                    }
                }
            }
            in_progress_requests.fetch_sub(1, Ordering::SeqCst);
        });
    }

    async fn send_block(
        block: &Block,
        reply_channel: &Sender<BehaviourSyncReply>,
    ) -> Result<(), ()> {
        let serialized_block = wire::serialize(block).map_err(|e| {
            tracing::error!("Serialization failed: {}", e);
        })?;
        let reply = BehaviourSyncReply::Block(serialized_block);
        reply_channel.send(reply).await.map_err(|_| {
            tracing::warn!("Reply channel closed");
        })?;
        sleep(Duration::from_millis(BLOCK_SEND_DELAY_MS)).await;
        Ok(())
    }

    async fn get_blocks_from_slot(
        storage_relay: &StorageRelay<Storage, RuntimeServiceId>,
        slot: Slot,
    ) -> Result<tokio::sync::mpsc::Receiver<ScanResult<Storage>>, DynError> {
        let prefix = Bytes::copy_from_slice(BLOCK_INDEX_PREFIX);
        let start = Bytes::copy_from_slice(&slot.to_be_bytes());
        let (msg, receiver) = StorageMsg::new_scan_range_message(prefix, start);
        storage_relay
            .send(msg)
            .await
            .map_err(|(e, _)| Box::new(e) as DynError)?;

        let result_receiver = receiver
            .into_inner()
            .await
            .map_err(|e| Box::new(e) as DynError)?;

        Ok(result_receiver)
    }

    async fn load_block(
        storage_relay: &OutboundRelay<
            <StorageService<Storage, RuntimeServiceId> as ServiceData>::Message,
        >,
        key: Bytes,
    ) -> Result<Option<Bytes>, DynError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        storage_relay
            .send(StorageMsg::Load {
                key,
                reply_channel: reply_tx,
            })
            .await
            .map_err(|(e, _)| Box::new(e) as DynError)?;
        reply_rx.await.map_err(|e| Box::new(e) as DynError)
    }
}
