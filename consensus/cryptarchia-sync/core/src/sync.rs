use std::{
    collections::{HashMap, HashSet},
    error::Error,
};

use cryptarchia_engine::Slot;
use futures::{stream::BoxStream, StreamExt};
use itertools::Itertools;
use nomos_core::{block::AbstractBlock, header::HeaderId};
use tracing::{debug, info};

use crate::{fetcher, CryptarchiaSyncError, CryptarchiaSyncExt};

#[async_trait::async_trait]
pub trait CryptarchiaSyncExtInternal<BlockFetcher>: CryptarchiaSyncExt
where
    Self::Block: Send,
    BlockFetcher: fetcher::BlockFetcher<Block = Self::Block> + Sync,
{
    /// Start a sync process.
    async fn run<'a>(
        mut self,
        block_fetcher: &'a BlockFetcher,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Start the sync process from the current local tip slot.
        let mut start_slot = self.tip_slot() + 1;
        info!("Starting sync process from slot {start_slot:?}");

        let mut rejected_blocks = HashSet::new();
        // Run a forward sync, and trigger backfillings if orphan blocks are found.
        // Repeat the forward sync if any backfilling was performed,
        // in order to catch up with the updated peers.
        loop {
            let ForwardSyncOutput {
                max_fetched_slot,
                orphan_blocks,
            } = self
                .sync_forward(start_slot, block_fetcher, &mut rejected_blocks)
                .await?;

            // Finish the sync process if no block ahead of start_slot was fetched.
            if let Some(max_fetched_slot) = max_fetched_slot {
                if max_fetched_slot < start_slot {
                    info!("No useful blocks fetched, finishing sync process");
                    return Ok(self);
                }
                info!("Fetched blocks up to slot {max_fetched_slot:?}");
                start_slot = max_fetched_slot + 1;
            } else {
                info!("No blocks fetched, finishing sync process");
                return Ok(self);
            }

            // Backfill the orphan forks.
            // Sort the orphan blocks by slot in descending order to minimize backfillings.
            for (orphan_block, provider_id) in orphan_blocks
                .iter()
                .sorted_by_key(|&(_, (slot, _))| std::cmp::Reverse(slot))
                .map(|(id, (_, provider_id))| (*id, provider_id.clone()))
            {
                // Skip the orphan block if it has been processed or rejected
                // during the previous backfillings.
                if self.has_block(&orphan_block) || rejected_blocks.contains(&orphan_block) {
                    continue;
                }

                match self
                    .backfill_fork(orphan_block, provider_id, block_fetcher)
                    .await
                {
                    Ok(()) => {}
                    Err(BackfillError::InvalidBlock {
                        error,
                        invalid_suffix,
                    }) => {
                        debug!("Invalid blocks found during a backfilling: {error:?}");
                        rejected_blocks.extend(invalid_suffix);
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }
            }
        }
    }

    /// Start a forward synchronization:
    /// - Fetch blocks from the peers, starting from the current local tip slot.
    /// - Process blocks.
    /// - Gather orphaned blocks whose parent is not in the local block tree.
    async fn sync_forward(
        &mut self,
        start_slot: Slot,
        block_fetcher: &BlockFetcher,
        rejected_blocks: &mut HashSet<HeaderId>,
    ) -> Result<ForwardSyncOutput<BlockFetcher>, Box<dyn Error + Send + Sync>> {
        let mut max_fetched_slot: Option<Slot> = None;
        let mut orphan_blocks = HashMap::new();

        info!("Fetching blocks from slot {:?}", start_slot);
        let mut stream = block_fetcher.fetch_blocks_forward(start_slot).await?;
        while let Some((block, provider_id)) = stream.next().await {
            // Update max_fetched_slot if necessary.
            max_fetched_slot = max_fetched_slot
                .map(|s| s.max(block.slot()))
                .or_else(|| Some(block.slot()));

            // Reject blocks that have been rejected in the past
            // or whose parent has been rejected.
            let id = block.id();
            let parent = block.parent();
            if rejected_blocks.contains(&id) || rejected_blocks.contains(&parent) {
                rejected_blocks.insert(id);
                continue;
            }

            let slot = block.slot();
            match self.process_block(block).await {
                Ok(()) => {
                    debug!("Processed block {id:?}");
                    orphan_blocks.remove(&id);
                }
                Err(CryptarchiaSyncError::ParentNotFound) => {
                    debug!("Parent not found for block {id:?}");
                    orphan_blocks.insert(id, (slot, provider_id));
                }
                Err(CryptarchiaSyncError::InvalidBlock(e)) => {
                    debug!("Invalid block {id:?}: {e}");
                    rejected_blocks.insert(id);
                }
            };
        }

        info!("Forward sync is done");
        Ok(ForwardSyncOutput {
            max_fetched_slot,
            orphan_blocks,
        })
    }

    /// Backfills a fork by fetching blocks from the peers.
    /// The fork choice rule is continuously applied during backfilling.
    async fn backfill_fork(
        &mut self,
        tip: HeaderId,
        provider_id: BlockFetcher::ProviderId,
        network: &BlockFetcher,
    ) -> Result<(), BackfillError> {
        let suffix = self
            .find_missing_part(
                network
                    .fetch_chain_backward(tip, provider_id)
                    .await
                    .map_err(BackfillError::BlockFetcherError)?,
            )
            .await;

        // Add blocks in the fork suffix with applying fork choice rule.
        // If an invalid block is found, return an error with all subsequent blocks.
        let mut iter = suffix.into_iter();
        while let Some(block) = iter.next() {
            let id = block.id();
            if let Err(error) = self.process_block(block).await {
                return Err(BackfillError::InvalidBlock {
                    error,
                    invalid_suffix: std::iter::once(id)
                        .chain(iter.map(|block| block.id()))
                        .collect(),
                });
            };
        }

        Ok(())
    }

    /// Fetch the fork until it reaches the local block tree.
    async fn find_missing_part<'a>(&self, fork: BoxStream<'a, Self::Block>) -> Vec<Self::Block> {
        let mut suffix = fork.collect::<Vec<_>>().await;
        suffix.reverse();
        suffix
    }
}

impl<T, BlockFetcher> CryptarchiaSyncExtInternal<BlockFetcher> for T
where
    T: CryptarchiaSyncExt,
    Self::Block: Send,
    BlockFetcher: fetcher::BlockFetcher<Block = Self::Block> + Sync,
{
}

pub struct ForwardSyncOutput<BlockFetcher: fetcher::BlockFetcher> {
    max_fetched_slot: Option<Slot>,
    orphan_blocks: HashMap<HeaderId, (Slot, BlockFetcher::ProviderId)>,
}

#[derive(thiserror::Error, Debug)]
pub enum BackfillError {
    #[error("Invalid block found during backfilling: {error:?}")]
    InvalidBlock {
        error: CryptarchiaSyncError,
        invalid_suffix: Vec<HeaderId>,
    },
    #[error("Block fetcher error during backfilling: {0}")]
    BlockFetcherError(Box<dyn Error + Send + Sync>),
}
