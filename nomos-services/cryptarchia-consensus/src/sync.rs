use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Debug,
    hash::Hash,
    marker::{PhantomData, Unpin},
};

use futures::{Stream, StreamExt};
use itertools::Itertools;
use nomos_core::{
    block::Block,
    da::blob::{info::DispersedBlobInfo, BlobSelect},
    header::HeaderId,
    tx::{Transaction, TxSelect},
};
use nomos_da_sampling::backend::DaSamplingServiceBackend;
use nomos_mempool::backend::RecoverableMempool;
use nomos_storage::backends::StorageBackend;
use rand::{RngCore, SeedableRng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    blend, leadership::Leader, network, relays::CryptarchiaConsensusRelays, Cryptarchia,
    CryptarchiaConsensus, Error,
};

#[expect(
    clippy::type_complexity,
    reason = "CryptarchiaConsensusRelays amount of generics."
)]
pub struct Synchronization<
    NetAdapter,
    BlendAdapter,
    ClPool,
    ClPoolAdapter,
    DaPool,
    DaPoolAdapter,
    TxS,
    BS,
    Storage,
    SamplingBackend,
    SamplingNetworkAdapter,
    SamplingRng,
    SamplingStorage,
    DaVerifierBackend,
    DaVerifierNetwork,
    DaVerifierStorage,
    TimeBackend,
    ApiAdapter,
    RuntimeServiceId,
> {
    _marker: PhantomData<(
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    )>,
}
impl<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
    Synchronization<
        NetAdapter,
        BlendAdapter,
        ClPool,
        ClPoolAdapter,
        DaPool,
        DaPoolAdapter,
        TxS,
        BS,
        Storage,
        SamplingBackend,
        SamplingNetworkAdapter,
        SamplingRng,
        SamplingStorage,
        DaVerifierBackend,
        DaVerifierNetwork,
        DaVerifierStorage,
        TimeBackend,
        ApiAdapter,
        RuntimeServiceId,
    >
where
    NetAdapter: network::NetworkAdapter<RuntimeServiceId, Tx = ClPool::Item, BlobCertificate = DaPool::Item>
        + Clone
        + Send
        + Sync
        + 'static,
    NetAdapter::Settings: Send,
    BlendAdapter: blend::BlendAdapter<RuntimeServiceId> + Clone + Send + Sync + 'static,
    BlendAdapter::Settings: Send,
    ClPool: RecoverableMempool<BlockId = HeaderId> + Send + Sync + 'static,
    ClPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    ClPool::Settings: Clone + Send + Sync + 'static,
    ClPool::Item: Transaction<Hash = ClPool::Key>
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
    ClPool::Key: Debug + Send + Sync,
    ClPoolAdapter: nomos_mempool::network::NetworkAdapter<
            RuntimeServiceId,
            Payload = ClPool::Item,
            Key = ClPool::Key,
        > + Send
        + Sync
        + 'static,
    DaPool::Item: DispersedBlobInfo<BlobId = DaPool::Key>
        + nomos_core::da::blob::metadata::Metadata
        + Debug
        + Clone
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    DaPool: RecoverableMempool<BlockId = HeaderId, Key = SamplingBackend::BlobId>
        + Send
        + Sync
        + 'static,
    DaPool::RecoveryState: Serialize + for<'de> Deserialize<'de>,
    DaPool::Settings: Clone + Send + Sync + 'static,
    DaPoolAdapter: nomos_mempool::network::NetworkAdapter<RuntimeServiceId, Key = DaPool::Key>
        + Send
        + Sync
        + 'static,
    DaPoolAdapter::Payload: DispersedBlobInfo + Into<DaPool::Item> + Debug,
    TxS: TxSelect<Tx = ClPool::Item> + Clone + Send + Sync + 'static,
    TxS::Settings: Send,
    BS: BlobSelect<BlobId = DaPool::Item> + Clone + Send + Sync + 'static,
    BS::Settings: Send,
    Storage: StorageBackend + Send + Sync + 'static,
    SamplingBackend: DaSamplingServiceBackend<SamplingRng> + Send,
    SamplingBackend::Settings: Clone,
    SamplingBackend::Share: Debug + 'static,
    SamplingBackend::BlobId: Debug + Ord + Send + Sync + 'static,
    SamplingNetworkAdapter: nomos_da_sampling::network::NetworkAdapter<RuntimeServiceId>,
    SamplingRng: SeedableRng + RngCore,
    SamplingStorage: nomos_da_sampling::storage::DaStorageAdapter<RuntimeServiceId>,
    DaVerifierStorage: nomos_da_verifier::storage::DaStorageAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierBackend: nomos_da_verifier::backend::VerifierBackend + Send + Sync + 'static,
    DaVerifierBackend::Settings: Clone,
    DaVerifierNetwork: nomos_da_verifier::network::NetworkAdapter<RuntimeServiceId> + Send + Sync,
    DaVerifierNetwork::Settings: Clone,
    TimeBackend: nomos_time::backends::TimeBackend,
    TimeBackend::Settings: Clone + Send + Sync,
    ApiAdapter: nomos_da_sampling::api::ApiAdapter + Send + Sync,
{
    /// Syncs the local block tree with the peers, starting from the local tip.
    /// This covers the case where the local tip is not on the latest honest
    /// chain anymore.
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    pub async fn initiate(
        cryptarchia: Cryptarchia,
        leader: &mut Leader,
        _ledger_config: nomos_ledger::Config,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        network_adapter: &NetAdapter,
    ) -> Cryptarchia {
        let mut cryptarchia = Some(cryptarchia);

        // Repeat the sync process until no peer has a tip ahead of the local tip,
        // because peers' tips may advance during the sync process.
        let mut rejected_blocks = HashSet::new();
        loop {
            // Fetch blocks from the peers in the range of slots from the local tip to the
            // latest tip. Gather orphaned blocks, which are blocks from forks
            // that are absent in the local block tree.
            let mut orphan_blocks = HashMap::new();
            let mut num_blocks = 0;

            // TODO: handle network error
            let mut stream = network_adapter
                .fetch_blocks_from_slot(cryptarchia.as_ref().unwrap().tip_state().slot())
                .await
                .unwrap();
            while let Some(block) = stream.next().await {
                num_blocks += 1;
                // Reject blocks that have been rejected in the past
                // or whose parent has been rejected.
                let header = block.header().clone();
                let id = header.id();
                if rejected_blocks.contains(&id) || rejected_blocks.contains(&header.parent()) {
                    rejected_blocks.insert(id);
                    continue;
                }

                match Self::process_block(
                    cryptarchia.take().unwrap(),
                    leader,
                    block,
                    relays,
                    block_broadcaster,
                )
                .await
                {
                    Ok(c) => {
                        orphan_blocks.remove(&id);
                        cryptarchia = Some(c);
                    }
                    Err((
                        Error::Ledger(nomos_ledger::LedgerError::ParentNotFound(_))
                        | Error::Consensus(cryptarchia_engine::Error::ParentMissing(_)),
                        c,
                    )) => {
                        orphan_blocks.insert(id, header.slot());
                        cryptarchia = Some(c);
                    }
                    Err((_, c)) => {
                        rejected_blocks.insert(id);
                        cryptarchia = Some(c);
                    }
                };
            }

            // Finish the sync process if no block has been fetched,
            // which means that no peer has a tip ahead of the local tip.
            if num_blocks == 0 {
                break;
            }

            // Backfill the orphan forks starting from the orphan blocks with applying fork
            // choice rule. Sort the orphan blocks by slot in descending order
            // to minimize the number of backfillings.
            for orphan_block in orphan_blocks
                .iter()
                .sorted_by_key(|&(_, slot)| std::cmp::Reverse(slot))
                .map(|(id, _)| *id)
            {
                // Skip the orphan block if it has been processed during the previous
                // backfillings (i.e. if it has been already added to the local
                // block tree). Or, skip if it has been rejected during the
                // previous backfillings.
                if cryptarchia
                    .as_ref()
                    .unwrap()
                    .ledger_state(&orphan_block)
                    .is_some()
                    || rejected_blocks.contains(&orphan_block)
                {
                    continue;
                }

                match Self::backfill_fork(
                    cryptarchia.take().unwrap(),
                    leader,
                    orphan_block,
                    relays,
                    block_broadcaster,
                    network_adapter,
                )
                .await
                {
                    Ok(c) => {
                        cryptarchia = Some(c);
                    }
                    Err((_, c, invalid_suffix)) => {
                        rejected_blocks.extend(invalid_suffix);
                        cryptarchia = Some(c);
                    }
                }
            }
        }

        cryptarchia.unwrap()
    }

    /// Backfills a fork, which is absent in the local block tree
    /// by fetching blocks from the peers.
    /// During backfilling, the fork choice rule is continuously applied.
    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    async fn backfill_fork(
        cryptarchia: Cryptarchia,
        leader: &mut Leader,
        fork_tip: HeaderId,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
        network_adapter: &NetAdapter,
    ) -> Result<Cryptarchia, (Error, Cryptarchia, Vec<HeaderId>)> {
        let mut cryptarchia = Some(cryptarchia);

        let suffix = Self::find_missing_part(
            // TODO: handle network error
            network_adapter
                .fetch_chain_backward(fork_tip)
                .await
                .unwrap(),
            cryptarchia.as_ref().unwrap(),
        )
        .await;

        // Add blocks in the fork suffix with applying fork choice rule.
        // After all, add the tip of the fork suffix to apply the fork choice rule.
        let mut iter = suffix.into_iter();
        while let Some(block) = iter.next() {
            let id = block.header().id();
            match Self::process_block(
                cryptarchia.take().unwrap(),
                leader,
                block,
                relays,
                block_broadcaster,
            )
            .await
            {
                Ok(c) => {
                    cryptarchia = Some(c);
                }
                Err((e, c)) => {
                    return Err((
                        e,
                        c,
                        std::iter::once(id)
                            .chain(iter.map(|block| block.header().id()))
                            .collect(),
                    ));
                }
            };
        }

        Ok(cryptarchia.unwrap())
    }

    /// Finds the point where the fork is disconnected from the local block
    /// tree, and returns the suffix of the fork from the disconnected point
    /// to the tip. The disconnected point may be different from the
    /// divergence point of the fork in the case where the fork has been
    /// partially backfilled.
    async fn find_missing_part(
        mut fork: Box<dyn Stream<Item = Block<ClPool::Item, DaPool::Item>> + Send + Sync + Unpin>,
        cryptarchia: &Cryptarchia,
    ) -> VecDeque<Block<ClPool::Item, DaPool::Item>> {
        let mut suffix = VecDeque::new();
        while let Some(block) = fork.next().await {
            if cryptarchia.ledger_state(&block.header().id()).is_none() {
                suffix.push_front(block);
            } else {
                break;
            }
        }
        suffix
    }

    #[expect(
        clippy::type_complexity,
        reason = "CryptarchiaConsensusRelays amount of generics."
    )]
    async fn process_block(
        cryptarchia: Cryptarchia,
        leader: &mut Leader,
        block: Block<ClPool::Item, DaPool::Item>,
        relays: &CryptarchiaConsensusRelays<
            BlendAdapter,
            BS,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            NetAdapter,
            SamplingBackend,
            SamplingRng,
            Storage,
            TxS,
            DaVerifierBackend,
            RuntimeServiceId,
        >,
        block_broadcaster: &mut broadcast::Sender<Block<ClPool::Item, DaPool::Item>>,
    ) -> Result<Cryptarchia, (Error, Cryptarchia)> {
        CryptarchiaConsensus::<
            NetAdapter,
            BlendAdapter,
            ClPool,
            ClPoolAdapter,
            DaPool,
            DaPoolAdapter,
            TxS,
            BS,
            Storage,
            SamplingBackend,
            SamplingNetworkAdapter,
            SamplingRng,
            SamplingStorage,
            DaVerifierBackend,
            DaVerifierNetwork,
            DaVerifierStorage,
            TimeBackend,
            ApiAdapter,
            RuntimeServiceId,
        >::process_block(cryptarchia, leader, block, relays, block_broadcaster)
        .await
    }
}
