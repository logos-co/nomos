use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
};

use cryptarchia_engine::Slot;
use futures::{Stream, StreamExt};
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
    async fn run(
        mut self,
        block_fetcher: &BlockFetcher,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        info!("Starting sync process from tip {:?}", self.tip_slot());
        let mut rejected_blocks = HashSet::new();
        // Run a forward sync, and trigger backfillings if orphan blocks are found.
        // Repeat the forward sync if any backfilling was performed,
        // in order to catch up with the updated peers.
        loop {
            let orphan_blocks = self
                .sync_forward(block_fetcher, &mut rejected_blocks)
                .await?;

            // Finish the sync process if there is no orphan block to be backfilled.
            // If not, perform backfillings and try the forward sync again
            // to catch up with the updated peers.
            if orphan_blocks.is_empty() {
                info!("No new blocks processed");
                break;
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

        info!("Finished sync process with tip {:?}", self.tip_slot());

        Ok(self)
    }

    /// Start a forward synchronization:
    /// - Fetch blocks from the peers, starting from the current local tip slot.
    /// - Process blocks.
    /// - Gather orphaned blocks whose parent is not in the local block tree.
    async fn sync_forward(
        &mut self,
        block_fetcher: &BlockFetcher,
        rejected_blocks: &mut HashSet<HeaderId>,
    ) -> Result<HashMap<HeaderId, (Slot, BlockFetcher::ProviderId)>, Box<dyn Error + Send + Sync>>
    {
        let mut orphan_blocks = HashMap::new();

        let start_slot = self.tip_slot() + 1;
        info!("Fetching blocks from slot {:?}", start_slot);
        let mut stream = block_fetcher.fetch_blocks_forward(start_slot).await?;

        while let Some((block, provider_id)) = stream.next().await {
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
        Ok(orphan_blocks)
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
    async fn find_missing_part(
        &self,
        mut fork: Box<dyn Stream<Item = Self::Block> + Send + Sync + Unpin>,
    ) -> VecDeque<Self::Block> {
        let mut suffix = VecDeque::new();
        while let Some(block) = fork.next().await {
            if self.has_block(&block.id()) {
                break;
            }
            suffix.push_front(block);
        }
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
