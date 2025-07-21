use std::{collections::HashSet, fmt::Debug};

use nomos_blend_service::network::NetworkAdapter as BlendNetworkAdapter;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::HeaderId,
    mantle::{Transaction, TxSelect},
};
use nomos_da_sampling::{backend::DaSamplingServiceBackend, DaSamplingServiceMsg};
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
    MempoolMsg,
};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend};
use overwatch::services::relay::OutboundRelay;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, error, instrument};

use crate::{
    blend, get_sampled_blobs, network,
    relays::CryptarchiaConsensusRelays,
    states::CryptarchiaConsensusStateUpdater,
    storage::{adapters::StorageAdapter, StorageAdapter as _, StorageAdapterExt as _},
    Cryptarchia, Error, LOG_TARGET,
};

#[async_trait::async_trait]
pub trait BlockProcessor {
    type Block;
    type ServiceStateUpdater;

    async fn process_block<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
        service_state_updater: Option<&Self::ServiceStateUpdater>,
    ) -> Result<Cryptarchia<CryptarchiaState>, (Error, Cryptarchia<CryptarchiaState>)>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send;
}

#[expect(
    clippy::type_complexity,
    reason = "CryptarchiaConsensusRelays amount of generics."
)]
pub struct NomosBlockProcessor<
    'a,
    BlendAdapter,
    BS,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    NetworkAdapter,
    SamplingBackend,
    Storage,
    TxS,
    DaVerifierBackend,
    RuntimeServiceId,
> where
    BlendAdapter:
        blend::BlendAdapter<RuntimeServiceId, Network: BlendNetworkAdapter<RuntimeServiceId>>,
    BS: BlobSelect,
    ClPool: MemPool,
    ClPool::Item: Clone + Eq,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    DaPool: MemPool,
    DaPool::Item: Clone + Eq,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId>,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend,
    TxS: TxSelect,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend,
{
    storage: &'a StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
    relays: &'a CryptarchiaConsensusRelays<
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >,
    block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    storage_blocks_to_remove: HashSet<HeaderId>,
}

#[async_trait::async_trait]
impl<
        'a,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    > BlockProcessor
    for NomosBlockProcessor<
        'a,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >
where
    BlendAdapter:
        blend::BlendAdapter<RuntimeServiceId, Network: BlendNetworkAdapter<RuntimeServiceId>>,
    BlendAdapter::Settings: Send + Sync,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send + Sync,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + 'static,
    ClPool::Settings: Clone + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::BlockId: Debug,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool::Key: Ord + Debug + Send + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Sync,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
{
    type Block = Block<ClPool::Item, DaPool::Item>;
    type ServiceStateUpdater = CryptarchiaConsensusStateUpdater<
        'a,
        TxS::Settings,
        BS::Settings,
        NetworkAdapter::Settings,
        BlendAdapter::Settings,
    >;

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// The updated [`Cryptarchia`] is returned if the block is valid and added
    /// successfully. If not, [`Error`] is returned along with the unchanged
    /// [`Cryptarchia`].
    #[instrument(level = "debug", skip(self, cryptarchia, service_state_updater))]
    async fn process_block<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
        service_state_updater: Option<&Self::ServiceStateUpdater>,
    ) -> Result<Cryptarchia<CryptarchiaState>, (Error, Cryptarchia<CryptarchiaState>)>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
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

impl<
        'a,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >
    NomosBlockProcessor<
        'a,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >
where
    BlendAdapter:
        blend::BlendAdapter<RuntimeServiceId, Network: BlendNetworkAdapter<RuntimeServiceId>>,
    BlendAdapter::Settings: Send + Sync,
    BS: BlobSelect<BlobId = DaPool::Item>,
    BS::Settings: Send + Sync,
    ClPool: RecoverableMempool<BlockId = HeaderId>,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + 'static,
    ClPool::Settings: Clone + Sync,
    ClPoolAdapter: MempoolAdapter<RuntimeServiceId, Payload = ClPool::Item, Key = ClPool::Key>,
    DaPool: RecoverableMempool<BlockId = HeaderId>,
    DaPool::BlockId: Debug,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + BlobMetadata
        + Debug
        + Clone
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool::Key: Ord + Debug + Send + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Sync,
    DaPoolAdapter: MempoolAdapter<RuntimeServiceId, Key = DaPool::Key>,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    NetworkAdapter: network::NetworkAdapter<RuntimeServiceId>,
    NetworkAdapter::Settings: Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
{
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    pub const fn new(
        storage: &'a StorageAdapter<Storage, ClPool::Item, DaPool::Item, RuntimeServiceId>,
        relays: &'a CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetworkAdapter,
            SamplingBackend,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Self {
        Self {
            storage,
            relays,
            block_subscription_sender,
            storage_blocks_to_remove,
        }
    }

    async fn validate_blobs(&self, block: &Block<ClPool::Item, DaPool::Item>) -> Result<(), Error> {
        let sampled_blob_ids = get_sampled_blobs(self.relays.sampling_relay())
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
            self.relays.cl_mempool_relay(),
            block.transactions().map(Transaction::hash),
            block.header().id(),
        )
        .await;
        Self::mark_in_block(
            self.relays.da_mempool_relay(),
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
            .relays
            .sampling_relay()
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

        self.storage_blocks_to_remove = self
            .storage
            .remove_blocks_and_collect_failures(
                self.storage_blocks_to_remove
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

    fn update_service_state<CryptarchiaState>(
        &self,
        cryptarchia: &Cryptarchia<CryptarchiaState>,
        updater: &CryptarchiaConsensusStateUpdater<
            '_,
            TxS::Settings,
            BS::Settings,
            NetworkAdapter::Settings,
            BlendAdapter::Settings,
        >,
    ) where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState,
    {
        if let Err(e) = updater.update(cryptarchia, self.storage_blocks_to_remove.clone()) {
            error!(
                target: LOG_TARGET,
                "Failed to update service state: {e}"
            );
        }
    }
}
