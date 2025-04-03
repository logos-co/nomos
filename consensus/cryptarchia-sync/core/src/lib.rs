pub mod adapter;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    marker::{PhantomData, Unpin},
};

use adapter::{CryptarchiaAdapter, CryptarchiaAdapterError, NetworkAdapter};
use futures::{Stream, StreamExt};
use itertools::Itertools;
use nomos_core::block::AbstractBlock;

pub struct Synchronization<Cryptarchia, Network, Block> {
    _marker: PhantomData<(Cryptarchia, Network, Block)>,
}

impl<Cryptarchia, Network, Block> Synchronization<Cryptarchia, Network, Block>
where
    Block: AbstractBlock + Send,
    Block::Id: Send + Sync,
    Cryptarchia: CryptarchiaAdapter<Block = Block> + Sync + Send,
    Network: NetworkAdapter<Block = Block> + Sync,
{
    /// Syncs the local block tree with the peers, starting from the local tip.
    /// This covers the case where the local tip is not on the latest honest
    /// chain anymore.
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
                let id = block.id();
                let parent = block.parent();
                if rejected_blocks.contains(&id) || rejected_blocks.contains(&parent) {
                    rejected_blocks.insert(id);
                    continue;
                }

                let slot = block.slot();
                match cryptarchia.process_block(block).await {
                    Ok(()) => {
                        orphan_blocks.remove(&id);
                    }
                    Err(CryptarchiaAdapterError::ParentNotFound) => {
                        orphan_blocks.insert(id, slot);
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
        tip: Block::Id,
        network: &Network,
    ) -> Result<(), (CryptarchiaAdapterError, Vec<Block::Id>)> {
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
            let id = block.id();
            if let Err(e) = cryptarchia.process_block(block).await {
                return Err((
                    e,
                    std::iter::once(id)
                        .chain(iter.map(|block| block.id()))
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
        mut fork: Box<dyn Stream<Item = Block> + Send + Sync + Unpin>,
        cryptarchia: &Cryptarchia,
    ) -> VecDeque<Block> {
        let mut suffix = VecDeque::new();
        while let Some(block) = fork.next().await {
            if cryptarchia.has_block(&block.id()) {
                break;
            }
            suffix.push_front(block);
        }
        suffix
    }
}
