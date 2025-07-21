use std::{collections::HashSet, fmt::Debug};

use nomos_core::{
    block::Block, da::blob::info::DispersedBlobInfo, header::HeaderId, mantle::Transaction,
};
use nomos_da_sampling::DaSamplingServiceMsg;
use nomos_mempool::{
    backend::MemPool, network::NetworkAdapter as MempoolNetworkAdapter, MempoolMsg,
};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend};
use overwatch::services::relay::OutboundRelay;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, error, instrument};

use crate::{
    get_sampled_blobs,
    relays::{ClMempoolRelay, DaMempoolRelay},
    states::{self},
    storage::{adapters::StorageAdapter, StorageAdapter as _, StorageAdapterExt as _},
    Cryptarchia, Error, SamplingRelay, LOG_TARGET,
};

#[async_trait::async_trait]
pub trait BlockProcessor {
    type Block;

    async fn process_block<CryptarchiaState, ServiceStateUpdater>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
        service_state_updater: Option<&ServiceStateUpdater>,
    ) -> Result<Cryptarchia<CryptarchiaState>, (Error, Cryptarchia<CryptarchiaState>)>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        ServiceStateUpdater: states::ServiceStateUpdater + Send + Sync;
}

pub struct NomosBlockProcessor<
    'a,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    Storage,
    RuntimeServiceId,
> where
    ClPool: MemPool,
    ClPool::Item: Clone + Eq,
    ClPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    DaPool: MemPool,
    DaPool::Item: Clone + Eq,
    DaPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
{
    storage: &'a StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
    cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
    da_mempool_relay: DaMempoolRelay<DaPool, DaPoolAdapter, DaPool::Key, RuntimeServiceId>,
    sampling_relay: SamplingRelay<DaPool::Key>,
    block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    stale_blocks: HashSet<HeaderId>,
}

#[async_trait::async_trait]
impl<ClPool, ClPoolAdapter, DaPool, DaPoolAdapter, Storage, RuntimeServiceId> BlockProcessor
    for NomosBlockProcessor<
        '_,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        Storage,
        RuntimeServiceId,
    >
where
    ClPool: MemPool,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Send,
    ClPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    DaPool: MemPool,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool::Key: Send + Ord + Debug,
    DaPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
{
    type Block = Block<ClPool::Item, DaPool::Item>;

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// The updated [`Cryptarchia`] is returned if the block is valid and added
    /// successfully. If not, [`Error`] is returned along with the unchanged
    /// [`Cryptarchia`].
    #[instrument(level = "debug", skip(self, cryptarchia, service_state_updater))]
    async fn process_block<CryptarchiaState, ServiceStateUpdater>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
        service_state_updater: Option<&ServiceStateUpdater>,
    ) -> Result<Cryptarchia<CryptarchiaState>, (Error, Cryptarchia<CryptarchiaState>)>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        ServiceStateUpdater: states::ServiceStateUpdater + Send + Sync,
    {
        debug!("processing a block proposal: {:?}", block);

        if let Err(e) = self.validate_blobs(&block).await {
            return Err((e, cryptarchia));
        }

        match cryptarchia.try_apply_header(block.header()) {
            Ok((cryptarchia, pruned_blocks)) => {
                self.mark_in_block_in_mempools(&block).await;
                self.mark_blobs_in_block(&block).await;
                self.update_block_storage(block.clone(), pruned_blocks.stale_blocks())
                    .await;
                self.notify_block_subscribers(block);
                if let Some(updater) = service_state_updater {
                    self.update_service_state(&cryptarchia, updater);
                }
                Ok(cryptarchia)
            }
            Err(
                e @ (Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(_))
                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(_))),
            ) => {
                // TODO: request parent block
                Err((e, cryptarchia))
            }
            Err(e) => Err((e, cryptarchia)),
        }
    }
}

impl<'a, ClPool, ClPoolAdapter, DaPool, DaPoolAdapter, Storage, RuntimeServiceId>
    NomosBlockProcessor<'a, ClPool, ClPoolAdapter, DaPool, DaPoolAdapter, Storage, RuntimeServiceId>
where
    ClPool: MemPool,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Send,
    ClPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    DaPool: MemPool,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool::Key: Send + Ord + Debug,
    DaPoolAdapter: MempoolNetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
{
    pub const fn new(
        storage: &'a StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
        cl_mempool_relay: ClMempoolRelay<ClPool, ClPoolAdapter, RuntimeServiceId>,
        da_mempool_relay: DaMempoolRelay<DaPool, DaPoolAdapter, DaPool::Key, RuntimeServiceId>,
        sampling_relay: SamplingRelay<DaPool::Key>,
        block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        stale_blocks: HashSet<HeaderId>,
    ) -> Self {
        Self {
            storage,
            cl_mempool_relay,
            da_mempool_relay,
            sampling_relay,
            block_subscription_sender,
            stale_blocks,
        }
    }

    async fn validate_blobs(&self, block: &Block<ClPool::Item, DaPool::Item>) -> Result<(), Error> {
        let sampled_blob_ids = get_sampled_blobs(&self.sampling_relay)
            .await
            .map_err(Error::BlobValidationFailure)?;
        if !block
            .blobs()
            .all(|blob| sampled_blob_ids.contains(&blob.blob_id()))
        {
            return Err(Error::SampledBlobNotFound);
        }
        Ok(())
    }

    async fn mark_in_block_in_mempools(&self, block: &Block<ClPool::Item, DaPool::Item>) {
        Self::mark_in_block(
            &self.cl_mempool_relay,
            block.transactions().map(Transaction::hash),
            block.header().id(),
        )
        .await;
        Self::mark_in_block(
            &self.da_mempool_relay,
            block.blobs().map(DispersedBlobInfo::blob_id),
            block.header().id(),
        )
        .await;
    }

    async fn mark_in_block<Payload, Item, Key>(
        mempool: &OutboundRelay<MempoolMsg<HeaderId, Payload, Item, Key>>,
        ids: impl Iterator<Item = Key>,
        block: HeaderId,
    ) where
        Key: Send,
        Payload: Send,
    {
        mempool
            .send(MempoolMsg::MarkInBlock {
                ids: ids.collect(),
                block,
            })
            .await
            .unwrap_or_else(|(e, _)| error!("Could not mark items in block: {e}"));
    }

    async fn mark_blobs_in_block(&self, block: &Block<ClPool::Item, DaPool::Item>) {
        let blobs_id = block
            .blobs()
            .map(DispersedBlobInfo::blob_id)
            .collect::<Vec<_>>();
        if let Err((_e, DaSamplingServiceMsg::MarkInBlock { blobs_id })) = self
            .sampling_relay
            .send(DaSamplingServiceMsg::MarkInBlock { blobs_id })
            .await
        {
            error!("Error marking in block for blobs ids: {blobs_id:?}");
        }
    }

    async fn update_block_storage(
        &mut self,
        new_block: Block<ClPool::Item, DaPool::Item>,
        stale_blocks: impl Iterator<Item = &HeaderId> + Send,
    ) {
        if let Err(e) = self
            .storage
            .store_block(new_block.header().id(), new_block)
            .await
        {
            error!("Could not store block {e}");
        }

        self.stale_blocks = self
            .storage
            .remove_blocks_and_collect_failures(
                self.stale_blocks
                    .iter()
                    .copied()
                    .chain(stale_blocks.copied()),
            )
            .await;
    }

    fn notify_block_subscribers(&self, block: Block<ClPool::Item, DaPool::Item>) {
        if let Err(e) = self.block_subscription_sender.send(block) {
            error!("Could not notify block to services {e}");
        }
    }

    fn update_service_state<CryptarchiaState, ServiceStateUpdater>(
        &self,
        cryptarchia: &Cryptarchia<CryptarchiaState>,
        updater: &ServiceStateUpdater,
    ) where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState,
        ServiceStateUpdater: states::ServiceStateUpdater,
    {
        if let Err(e) = updater.update(cryptarchia, self.stale_blocks.clone()) {
            error!(
                target: LOG_TARGET,
                "Failed to update service state: {e}"
            );
        }
    }
}
