use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Debug,
    hash::Hash,
    marker::{PhantomData, Unpin},
};

use cryptarchia_engine::Slot;
use futures::{Stream, StreamExt};
use itertools::Itertools;
use nomos_core::{block::Block, header::HeaderId};
use serde::{de::DeserializeOwned, Serialize};

use crate::network;

// TODO: Consider moving this to a separate crate.
// (e.g. consensus/cryptarchia-sync/core)

// TODO: Replace Tx and BlobCertificate with Block if possible.
// This requires refactoring NetworkAdapter.
pub struct Synchronization<Cryptarchia, Network, Tx, BlobCertificate, RuntimeServiceId> {
    _marker: PhantomData<(Cryptarchia, Network, Tx, BlobCertificate, RuntimeServiceId)>,
}

impl<Cryptarchia, Network, Tx, BlobCertificate, RuntimeServiceId>
    Synchronization<Cryptarchia, Network, Tx, BlobCertificate, RuntimeServiceId>
where
    Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + 'static,
    BlobCertificate: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + 'static,
    Cryptarchia: CryptarchiaAdapter<Tx = Tx, BlobCertificate = BlobCertificate> + Sync + Send,
    Network: network::NetworkAdapter<RuntimeServiceId, Tx = Tx, BlobCertificate = BlobCertificate>
        + Sync,
{
    /// Syncs the local block tree with the peers, starting from the local tip.
    /// This covers the case where the local tip is not on the latest honest
    /// chain anymore.
    #[expect(dead_code, reason = "Sync protocol is not integrated yet.")]
    pub async fn run(
        mut cryptarchia: Cryptarchia,
        network: &Network,
    ) -> Result<Cryptarchia, Box<dyn std::error::Error + Send + Sync + 'static>> {
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
            let mut stream = network
                .fetch_blocks_from_slot(cryptarchia.tip_slot())
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

                match cryptarchia.process_block(block).await {
                    Ok(()) => {
                        orphan_blocks.remove(&id);
                    }
                    Err(CryptarchiaAdapterError::ParentNotFound) => {
                        orphan_blocks.insert(id, header.slot());
                    }
                    Err(CryptarchiaAdapterError::InvalidBlock(_)) => {
                        rejected_blocks.insert(id);
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
                if cryptarchia.has_block(&orphan_block) || rejected_blocks.contains(&orphan_block) {
                    continue;
                }

                if let Err((_, invalid_suffix)) =
                    Self::backfill_fork(&mut cryptarchia, orphan_block, network).await
                {
                    rejected_blocks.extend(invalid_suffix);
                }
            }
        }

        Ok(cryptarchia)
    }

    /// Backfills a fork, which is absent in the local block tree
    /// by fetching blocks from the peers.
    /// During backfilling, the fork choice rule is continuously applied.
    async fn backfill_fork(
        cryptarchia: &mut Cryptarchia,
        tip: HeaderId,
        network: &Network,
    ) -> Result<(), (CryptarchiaAdapterError, Vec<HeaderId>)> {
        let suffix = Self::find_missing_part(
            // TODO: handle network error
            network.fetch_chain_backward(tip).await.unwrap(),
            cryptarchia,
        )
        .await;

        // Add blocks in the fork suffix with applying fork choice rule.
        // After all, add the tip of the fork suffix to apply the fork choice rule.
        let mut iter = suffix.into_iter();
        while let Some(block) = iter.next() {
            let id = block.header().id();
            if let Err(e) = cryptarchia.process_block(block).await {
                return Err((
                    e,
                    std::iter::once(id)
                        .chain(iter.map(|block| block.header().id()))
                        .collect(),
                ));
            };
        }

        Ok(())
    }

    /// Finds the point where the fork is disconnected from the local block
    /// tree, and returns the suffix of the fork from the disconnected point
    /// to the tip. The disconnected point may be different from the
    /// divergence point of the fork in the case where the fork has been
    /// partially backfilled.
    async fn find_missing_part(
        mut fork: Box<dyn Stream<Item = Block<Tx, BlobCertificate>> + Send + Sync + Unpin>,
        cryptarchia: &Cryptarchia,
    ) -> VecDeque<Block<Tx, BlobCertificate>> {
        let mut suffix = VecDeque::new();
        while let Some(block) = fork.next().await {
            if cryptarchia.has_block(&block.header().id()) {
                break;
            }
            suffix.push_front(block);
        }
        suffix
    }
}

#[async_trait::async_trait]
pub trait CryptarchiaAdapter {
    // TODO: Replace Tx and BlobCertificate with Block.
    //       This requires refactoring NetworkAdapter.
    type Tx: Clone + Eq + Hash + 'static;
    type BlobCertificate: Clone + Eq + Hash + 'static;

    async fn process_block(
        &mut self,
        block: Block<Self::Tx, Self::BlobCertificate>,
    ) -> Result<(), CryptarchiaAdapterError>;

    fn tip_slot(&self) -> Slot;

    fn has_block(&self, id: &HeaderId) -> bool;
}

#[derive(thiserror::Error, Debug)]
pub enum CryptarchiaAdapterError {
    #[error("Parent not found")]
    ParentNotFound,
    #[error("Invalid block: {0}")]
    InvalidBlock(Box<dyn std::error::Error + Send + Sync>),
}
