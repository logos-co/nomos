use crate::network::SyncRequest;
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use nomos_core::block::Block;
use nomos_core::header::HeaderId;
use nomos_core::wire;
use nomos_mempool::backend::RecoverableMempool;
use nomos_network::backends::SyncRequestKind;
use nomos_storage::{StorageMsg, StorageService};
use overwatch::services::relay::OutboundRelay;
use overwatch::services::ServiceData;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::Sender;
use tokio::time::{sleep, Duration};

const MAX_CONCURRENT_REQUESTS: usize = 1;
const BLOCK_SEND_DELAY_MS: u64 = 50;

pub struct SyncBlocksProvider<
    Storage: nomos_storage::backends::StorageBackend + Send + Sync + 'static,
    ClPool,
    DaPool,
    RuntimeServiceId,
> {
    storage_relay:
        OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>,
    runtime: Handle,
    in_progress_requests: Arc<AtomicUsize>,
    phantom_data: PhantomData<(ClPool, DaPool)>,
}

impl<
        Storage: nomos_storage::backends::StorageBackend + Send + Sync + 'static,
        ClPool: RecoverableMempool,
        DaPool: RecoverableMempool,
        RuntimeServiceId,
    > SyncBlocksProvider<Storage, ClPool, DaPool, RuntimeServiceId>
where
    Storage: Send + Sync + 'static,
    ClPool: Send + Sync + 'static,
    ClPool::Item: Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
    DaPool: Send + Sync + 'static,
    DaPool::Item: Clone + Eq + Hash + Serialize + DeserializeOwned + Send + Sync + 'static,
{
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

    pub async fn process_sync_request(&self, request: SyncRequest) {
        if self.in_progress_requests.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_REQUESTS {
            tracing::warn!("Max concurrent sync requests reached");
            self.in_progress_requests.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        match request.kind {
            SyncRequestKind::ForwardChain(slot) => {
                self.spawn_fetch_blocks_from_slot_forwards(slot, request.reply_channel);
            }
            SyncRequestKind::BackwardChain(tip) => {
                self.spawn_fetch_blocks_from_header_backwards(tip.into(), request.reply_channel);
            }
            _ => {
                tracing::warn!("Unsupported sync request kind");
                self.in_progress_requests.fetch_sub(1, Ordering::SeqCst);
                return;
            }
        }
    }

    fn spawn_fetch_blocks_from_slot_forwards(
        &self,
        // Ignore for now for genesis sync
        _slot: u64,
        reply_channel: Sender<BehaviourSyncReply>,
    ) {
        let storage_relay = self.storage_relay.clone();
        let in_progress_requests = self.in_progress_requests.clone();
        self.runtime.spawn(async move {
            let mut epoch = 0;
            loop {
                let prefix = format!("blocks_epoch_{}", epoch);
                let (msg, receiver) = <StorageMsg<Storage>>::new_load_prefix_message(prefix);

                if let Err((e, _)) = storage_relay.send(msg).await {
                    tracing::error!("Failed to send storage message {}", e);
                } else {
                    match receiver.recv::<Block<ClPool::Item, DaPool::Item>>().await {
                        Ok(blocks) => {
                            if blocks.is_empty() {
                                break;
                            }
                            for block in blocks {
                                if let Err(_) = Self::send_block(&block, &reply_channel).await {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to receive blocks {}", e);
                        }
                    }
                }
                epoch += 1;
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
        let in_progress_requests = self.in_progress_requests.clone();
        self.runtime.spawn(async move {
            let mut current_header_id = header_id;
            loop {
                let (msg, receiver) = <StorageMsg<Storage>>::new_load_message(current_header_id);
                if let Err((e, _)) = storage_relay.send(msg).await {
                    tracing::error!("Failed to send storage message: {}", e);
                    break;
                }
                match receiver.recv::<Block<ClPool::Item, DaPool::Item>>().await {
                    Ok(Some(block)) => {
                        current_header_id = block.header().parent();
                        if let Err(_) = Self::send_block(&block, &reply_channel).await {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        tracing::error!("Failed to receive block: {}", e);
                        break;
                    }
                }
            }
            in_progress_requests.fetch_sub(1, Ordering::SeqCst);
        });
    }

    async fn send_block(
        block: &Block<ClPool::Item, DaPool::Item>,
        reply_channel: &Sender<BehaviourSyncReply>,
    ) -> Result<(), ()> {
        let serialized_block = wire::serialize(block).map_err(|e| {
            tracing::error!("Serialization failed: {}", e);
        })?;
        let reply = BehaviourSyncReply::Block(serialized_block);
        reply_channel.send(reply).await.map_err(|_| {
            tracing::warn!("Reply channel closed");
        })?;

        // Introduce a delay to avoid overwhelming everything else
        sleep(Duration::from_millis(BLOCK_SEND_DELAY_MS)).await;
        Ok(())
    }
}
