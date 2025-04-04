use crate::network::SyncRequest;
use cryptarchia_sync_network::behaviour::BehaviourSyncReply;
use cryptarchia_sync_network::SyncRequestKind;
use nomos_core::block::AbstractBlock;
use nomos_core::wire;
use nomos_storage::{StorageMsg, StorageService};
use overwatch::services::relay::OutboundRelay;
use overwatch::services::ServiceData;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::Sender;
use tokio::time::{sleep, Duration};
use tracing::info;

const MAX_CONCURRENT_REQUESTS: usize = 1;
const BLOCK_SEND_DELAY_MS: u64 = 50;

pub struct SyncBlocksProvider<
    Storage: nomos_storage::backends::StorageBackend + Send + Sync + 'static,
    Block,
    RuntimeServiceId,
> {
    storage_relay:
        OutboundRelay<<StorageService<Storage, RuntimeServiceId> as ServiceData>::Message>,
    runtime: Handle,
    in_progress_requests: Arc<AtomicUsize>,
    number_of_slots_per_epoch: u64,
    phantom_data: PhantomData<Block>,
}

impl<Storage, Block, RuntimeServiceId> SyncBlocksProvider<Storage, Block, RuntimeServiceId>
where
    Storage: nomos_storage::backends::StorageBackend + Send + Sync + 'static,
    Block: AbstractBlock + Serialize + DeserializeOwned + Send + Sync + 'static + std::fmt::Debug,
    <Block as AbstractBlock>::Id:
        From<[u8; 32]> + Serialize + DeserializeOwned + Hash + Eq + Send + Sync + 'static,
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
            number_of_slots_per_epoch: 100,
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
            SyncRequestKind::BackwardChain(tip) => {
                info!("Syncing from header {:?}", tip);
                self.spawn_fetch_blocks_from_header_backwards(tip.into(), request.reply_channel);
            }
            SyncRequestKind::Tip => {
                tracing::warn!("Unsupported sync request kind");
                self.in_progress_requests.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    fn spawn_fetch_blocks_from_slot_forwards(
        &self,
        slot: u64,
        reply_channel: Sender<BehaviourSyncReply>,
    ) {
        let storage_relay = self.storage_relay.clone();
        let in_progress_requests = Arc::<AtomicUsize>::clone(&self.in_progress_requests);
        let mut epoch = slot / self.number_of_slots_per_epoch;
        self.runtime.spawn(async move {
            loop {
                let prefix = format!("blocks_epoch_{epoch}").into_bytes();
                info!("Loading blocks from prefix: {:?}", prefix);

                let (msg, receiver) = <StorageMsg<Storage>>::new_load_prefix_message(prefix.into());

                if let Err((e, _)) = storage_relay.send(msg).await {
                    tracing::error!("Failed to send storage message {}", e);
                } else {
                    match receiver.recv::<Block>().await {
                        Ok(blocks) => {
                            if blocks.is_empty() {
                                tracing::debug!("No more blocks found for epoch {}", epoch);
                                break;
                            }
                            info!("Received blocks from database: {:?}", blocks.len());
                            for block in blocks {
                                if Self::send_block(&block, &reply_channel).await.is_err() {
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
        header_id: <Block as AbstractBlock>::Id,
        reply_channel: Sender<BehaviourSyncReply>,
    ) {
        let storage_relay = self.storage_relay.clone();
        let in_progress_requests = Arc::<AtomicUsize>::clone(&self.in_progress_requests);
        self.runtime.spawn(async move {
            let mut current_header_id = header_id;
            loop {
                let (msg, receiver) = <StorageMsg<Storage>>::new_load_message(current_header_id);
                if let Err((e, _)) = storage_relay.send(msg).await {
                    tracing::error!("Failed to send storage message: {}", e);
                    break;
                }
                match receiver.recv::<Block>().await {
                    Ok(Some(block)) => {
                        current_header_id = block.parent();
                        if Self::send_block(&block, &reply_channel).await.is_err() {
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

        // Introduce a delay to avoid overwhelming everything else
        sleep(Duration::from_millis(BLOCK_SEND_DELAY_MS)).await;
        Ok(())
    }
}
