use std::{
    collections::{BTreeSet, HashSet},
    fmt::Debug,
    hash::Hash,
};

use nomos_blend_service::network::NetworkAdapter as BlendNetworkAdapter;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, metadata::Metadata as BlobMetadata, BlobSelect},
    header::HeaderId,
    mantle::{Transaction, TxSelect},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::{
    backend::{MemPool, RecoverableMempool},
    network::NetworkAdapter as MempoolAdapter,
};
use nomos_storage::{api::chain::StorageChainApi, backends::StorageBackend};
use overwatch::services::state::StateUpdater;
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, error, instrument};

use crate::{
    blend, get_sampled_blobs,
    leadership::Leader,
    mark_blob_in_block, mark_in_block, network,
    relays::CryptarchiaConsensusRelays,
    states::CryptarchiaConsensusState,
    storage::{adapters::StorageAdapter, StorageAdapter as _, StorageAdapterExt as _},
    Cryptarchia, Error, LOG_TARGET,
};

#[async_trait::async_trait]
pub trait BlockProcessor {
    type Block;

    async fn process_block_and_update_service_state<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send;

    async fn process_block<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send;
}

#[expect(
    clippy::type_complexity,
    reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
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
    SamplingRng,
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
    SamplingRng: SeedableRng + RngCore,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng>,
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
        SamplingRng,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    >,
    block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    state_updater: StateUpdater<
        Option<
            CryptarchiaConsensusState<
                TxS::Settings,
                BS::Settings,
                NetworkAdapter::Settings,
                BlendAdapter::Settings,
                NetworkAdapter::PeerId,
            >,
        >,
    >,
    leader: &'a Leader,
    storage_blocks_to_remove: HashSet<HeaderId>,
}

#[async_trait::async_trait]
impl<
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        SamplingRng,
        Storage,
        TxS,
        DaVerifierBackend,
        RuntimeServiceId,
    > BlockProcessor
    for NomosBlockProcessor<
        '_,
        BlendAdapter,
        BS,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        NetworkAdapter,
        SamplingBackend,
        SamplingRng,
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
    NetworkAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
    Storage: StorageBackend + Send + Sync + 'static,
    <Storage as StorageChainApi>::Block:
        TryFrom<Block<ClPool::Item, DaPool::Item>> + TryInto<Block<ClPool::Item, DaPool::Item>>,
    TxS: TxSelect<Tx = ClPool::Item>,
    TxS::Settings: Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
{
    type Block = Block<ClPool::Item, DaPool::Item>;

    async fn process_block_and_update_service_state<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
    {
        let cryptarchia = self.process_block(cryptarchia, block).await;

        self.storage_blocks_to_remove = self
            .storage
            .remove_blocks_and_collect_failures(self.storage_blocks_to_remove.iter().copied())
            .await;

        match CryptarchiaConsensusState::<_, _, _, _, _>::from_cryptarchia_and_unpruned_blocks(
            &cryptarchia,
            self.leader,
            self.storage_blocks_to_remove.clone(),
        ) {
            Ok(state) => {
                self.state_updater.update(Some(state));
            }
            Err(e) => {
                error!(target: LOG_TARGET, "Failed to update state: {}", e);
            }
        }

        cryptarchia
    }

    /// Try to add a [`Block`] to [`Cryptarchia`].
    /// A [`Block`] is only added if it's valid
    #[instrument(level = "debug", skip(self, cryptarchia))]
    async fn process_block<CryptarchiaState>(
        &mut self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        block: Self::Block,
    ) -> Cryptarchia<CryptarchiaState>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
    {
        debug!("received proposal {:?}", block);

        let sampled_blobs = match get_sampled_blobs(self.relays.sampling_relay().clone()).await {
            Ok(sampled_blobs) => sampled_blobs,
            Err(error) => {
                error!("Unable to retrieved sampled blobs: {error}");
                return cryptarchia;
            }
        };
        if !Self::validate_blocks_blobs(&block, &sampled_blobs) {
            error!("Invalid block: {block:?}");
            return cryptarchia;
        }

        // TODO: filter on time?
        let header = block.header();
        let id = header.id();

        match cryptarchia.try_apply_header(header) {
            Ok((cryptarchia, pruned_blocks)) => {
                // remove included content from mempool
                mark_in_block(
                    self.relays.cl_mempool_relay().clone(),
                    block.transactions().map(Transaction::hash),
                    id,
                )
                .await;
                mark_in_block(
                    self.relays.da_mempool_relay().clone(),
                    block.blobs().map(DispersedBlobInfo::blob_id),
                    id,
                )
                .await;

                mark_blob_in_block(
                    self.relays.sampling_relay().clone(),
                    block.blobs().map(DispersedBlobInfo::blob_id).collect(),
                )
                .await;

                if let Err(e) = self.storage.store_block(header.id(), block.clone()).await {
                    error!("Could not store block {e}");
                }

                if let Err(e) = self.block_subscription_sender.send(block) {
                    error!("Could not notify block to services {e}");
                }

                if let Err(e) = self
                    .storage
                    .store_immutable_block_ids(pruned_blocks.immutable_blocks().copied())
                    .await
                {
                    error!("Could not store immutable blocks as immutable: {e}");
                }

                self.storage_blocks_to_remove
                    .extend(pruned_blocks.stale_blocks());
                return cryptarchia;
            }
            Err(
                Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(parent))
                | Error::Consensus(cryptarchia_engine::Error::ParentMissing(parent)),
            ) => {
                debug!("missing parent {:?}", parent);
                // TODO: request parent block
            }
            Err(e) => {
                debug!("invalid block {:?}: {e:?}", block);
            }
        }

        cryptarchia
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
        SamplingRng,
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
        SamplingRng,
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
    NetworkAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng, BlobId = DaPool::Key> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingRng: SeedableRng + RngCore,
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
        reason = "CryptarchiaConsensusState and CryptarchiaConsensusRelays amount of generics."
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
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_subscription_sender: broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        state_updater: StateUpdater<
            Option<
                CryptarchiaConsensusState<
                    TxS::Settings,
                    BS::Settings,
                    NetworkAdapter::Settings,
                    BlendAdapter::Settings,
                    NetworkAdapter::PeerId,
                >,
            >,
        >,
        leader: &'a Leader,
        storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Self {
        Self {
            storage,
            relays,
            block_subscription_sender,
            state_updater,
            leader,
            storage_blocks_to_remove,
        }
    }

    fn validate_blocks_blobs(
        block: &Block<ClPool::Item, DaPool::Item>,
        sampled_blobs_ids: &BTreeSet<DaPool::Key>,
    ) -> bool {
        let validated_blobs = block
            .blobs()
            .all(|blob| sampled_blobs_ids.contains(&blob.blob_id()));
        validated_blobs
    }

    pub fn add_storage_blocks_to_remove(&mut self, blocks: impl Iterator<Item = HeaderId>) {
        self.storage_blocks_to_remove.extend(blocks);
    }
}
