pub mod adapter;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
    marker::PhantomData,
};

use cryptarchia_engine::Slot;
use futures::{Stream, StreamExt};
use itertools::Itertools;
use nomos_core::{block::AbstractBlock, header::HeaderId};
use tracing::{debug, info};

use crate::adapter::CryptarchiaAdapterError;

pub struct Synchronization<Cryptarchia, BlockFetcher, Block> {
    _marker: PhantomData<(Cryptarchia, BlockFetcher, Block)>,
}

impl<Cryptarchia, BlockFetcher, Block> Synchronization<Cryptarchia, BlockFetcher, Block>
where
    Block: AbstractBlock + Send,
    Cryptarchia: adapter::CryptarchiaAdapter<Block = Block> + Sync + Send,
    BlockFetcher: adapter::BlockFetcher<Block = Block> + Sync,
{
    /// Syncs the local block tree with the peers, starting from the local tip.
    pub async fn run(
        cryptarchia: &mut Cryptarchia,
        block_fetcher: &BlockFetcher,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        info!(
            "Starting sync process from tip {:?}",
            cryptarchia.tip_slot()
        );

        let mut rejected_blocks = HashSet::new();
        // Run a forward sync, and trigger backfillings if orphan blocks are found.
        // Repeat the forward sync if any backfilling was performed,
        // in order to catch up with the updated peers.
        loop {
            let orphan_blocks =
                Self::sync_forward(cryptarchia, block_fetcher, &mut rejected_blocks).await?;

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
                if cryptarchia.has_block(&orphan_block) || rejected_blocks.contains(&orphan_block) {
                    continue;
                }

                match Self::backfill_fork(cryptarchia, orphan_block, provider_id, block_fetcher)
                    .await
                {
                    Ok(()) => {}
                    Err((e, Some(invalid_suffix))) => {
                        debug!("Invalid blocks found during a backfilling: {e:?}");
                        rejected_blocks.extend(invalid_suffix);
                    }
                    Err((e, None)) => {
                        return Err(e);
                    }
                }
            }
        }

        info!(
            "Finished sync process with tip {:?}",
            cryptarchia.tip_slot()
        );

        Ok(())
    }

    /// Start a forward synchronization:
    /// - Fetch blocks from the peers, starting from the current local tip slot.
    /// - Process blocks.
    /// - Gather orphaned blocks whose parent is not in the local block tree.
    async fn sync_forward(
        cryptarchia: &mut Cryptarchia,
        block_fetcher: &BlockFetcher,
        rejected_blocks: &mut HashSet<HeaderId>,
    ) -> Result<HashMap<HeaderId, (Slot, BlockFetcher::ProviderId)>, Box<dyn Error + Send + Sync>>
    {
        let mut orphan_blocks = HashMap::new();

        let start_slot = cryptarchia.tip_slot() + 1;
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
            match cryptarchia.process_block(block).await {
                Ok(()) => {
                    debug!("Processed block {id:?}");
                    orphan_blocks.remove(&id);
                }
                Err(CryptarchiaAdapterError::ParentNotFound) => {
                    debug!("Parent not found for block {id:?}");
                    orphan_blocks.insert(id, (slot, provider_id));
                }
                Err(CryptarchiaAdapterError::InvalidBlock(e)) => {
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
        cryptarchia: &mut Cryptarchia,
        tip: HeaderId,
        provider_id: BlockFetcher::ProviderId,
        network: &BlockFetcher,
    ) -> Result<(), (Box<dyn Error + Send + Sync>, Option<Vec<HeaderId>>)> {
        let suffix = Self::find_missing_part(
            network
                .fetch_chain_backward(tip, provider_id)
                .await
                .map_err(|e| (e, None))?,
            cryptarchia,
        )
        .await;

        // Add blocks in the fork suffix with applying fork choice rule.
        // If an invalid block is found, return an error with all subsequent blocks.
        let mut iter = suffix.into_iter();
        while let Some(block) = iter.next() {
            let id = block.id();
            if let Err(e) = cryptarchia.process_block(block).await {
                return Err((
                    Box::new(e),
                    Some(
                        std::iter::once(id)
                            .chain(iter.map(|block| block.id()))
                            .collect(),
                    ),
                ));
            };
        }

        Ok(())
    }

    /// Fetch the fork until it reaches the local block tree.
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::LazyLock};

    use cryptarchia_engine::Slot;
    use nomos_core::{block::AbstractBlock, header::HeaderId};

    use super::*;
    use crate::adapter::{BlockFetcher, BoxedStream, CryptarchiaAdapter, CryptarchiaAdapterError};

    #[tokio::test]
    async fn sync_single_chain_from_genesis() {
        // Prepare a peer with a single chain:
        // G - b1 - b2 - b3
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(i as u64),
                true,
            ))
            .unwrap();
        }
        assert_eq!(peer.honest_chain, HeaderId::from([3; 32]));
        assert!(peer.forks.is_empty());

        // Start a sync from genesis.
        // Result: The same block tree as the peer's
        let mut local = MockCryptarchia::new();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local, peer);
    }

    #[tokio::test]
    async fn sync_single_chain_from_middle() {
        // Prepare a peer with a single chain:
        // G - b1 - b2 - b3
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(i as u64),
                true,
            ))
            .unwrap();
        }
        assert_eq!(peer.honest_chain, HeaderId::from([3; 32]));
        assert!(peer.forks.is_empty());

        // Start a sync from a tree:
        // G - b1
        //
        // Result: The same block tree as the peer's
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&HeaderId::from([1; 32])).unwrap().clone())
            .unwrap();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local, peer);
    }

    #[tokio::test]
    async fn sync_forks_from_genesis() {
        // Prepare a peer with forks:
        // G - b1 - b2 - b5 == tip
        //   \
        //     b3 - b4
        let mut peer = MockCryptarchia::new();
        peer.process_block(MockBlock::new(
            HeaderId::from([1; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([2; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([3; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([4; 32]),
            HeaderId::from([3; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([5; 32]),
            HeaderId::from([2; 32]),
            Slot::from(3),
            true,
        ))
        .unwrap();
        assert_eq!(peer.honest_chain, HeaderId::from([5; 32]));
        assert_eq!(peer.forks, HashSet::from([HeaderId::from([4; 32])]));

        // Start a sync from genesis.
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local, peer);
    }

    #[tokio::test]
    async fn sync_forks_from_middle() {
        // Prepare a peer with forks:
        // G - b1 - b2 - b5 == tip
        //   \
        //     b3 - b4
        let mut peer = MockCryptarchia::new();
        peer.process_block(MockBlock::new(
            HeaderId::from([1; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([2; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([3; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([4; 32]),
            HeaderId::from([3; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([5; 32]),
            HeaderId::from([2; 32]),
            Slot::from(3),
            true,
        ))
        .unwrap();
        assert_eq!(peer.honest_chain, HeaderId::from([5; 32]));
        assert_eq!(peer.forks, HashSet::from([HeaderId::from([4; 32])]));

        // Start a sync from a tree:
        // G - b1
        //   \
        //     b3
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&HeaderId::from([1; 32])).unwrap().clone())
            .unwrap();
        local
            .process_block(peer.blocks.get(&HeaderId::from([3; 32])).unwrap().clone())
            .unwrap();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local, peer);
    }

    #[tokio::test]
    async fn sync_forks_by_backfilling() {
        // Prepare a peer with forks:
        // G - b1 - b2 - b5 == tip
        //   \
        //     b3 - b4
        let mut peer = MockCryptarchia::new();
        peer.process_block(MockBlock::new(
            HeaderId::from([1; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([2; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([3; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([4; 32]),
            HeaderId::from([3; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([5; 32]),
            HeaderId::from([2; 32]),
            Slot::from(3),
            true,
        ))
        .unwrap();
        assert_eq!(peer.honest_chain, HeaderId::from([5; 32]));
        assert_eq!(peer.forks, HashSet::from([HeaderId::from([4; 32])]));

        // Start a sync from a tree without the fork:
        // G - b1
        //
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&HeaderId::from([1; 32])).unwrap().clone())
            .unwrap();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local, peer);
    }

    #[tokio::test]
    async fn sync_multiple_peers_from_genesis() {
        // Prepare multiple peers:
        // Peer-0:                 b6
        //                        /
        // Peer-1: G - b1 - b2 - b3
        //                \
        // Peer-2:          b4 - b5
        let b1 = MockBlock::new(HeaderId::from([1; 32]), *GENESIS_ID, Slot::from(1), true);
        let b2 = MockBlock::new(
            HeaderId::from([2; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        );
        let b3 = MockBlock::new(
            HeaderId::from([3; 32]),
            HeaderId::from([2; 32]),
            Slot::from(3),
            true,
        );
        let b4 = MockBlock::new(
            HeaderId::from([4; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        );
        let b5 = MockBlock::new(
            HeaderId::from([5; 32]),
            HeaderId::from([4; 32]),
            Slot::from(3),
            true,
        );
        let b6 = MockBlock::new(
            HeaderId::from([6; 32]),
            HeaderId::from([3; 32]),
            Slot::from(4),
            true,
        );
        let mut peer0 = MockCryptarchia::new();
        peer0.process_block(b1.clone()).unwrap();
        peer0.process_block(b2.clone()).unwrap();
        peer0.process_block(b3.clone()).unwrap();
        peer0.process_block(b6.clone()).unwrap();
        let mut peer1 = MockCryptarchia::new();
        peer1.process_block(b1.clone()).unwrap();
        peer1.process_block(b2.clone()).unwrap();
        peer1.process_block(b3.clone()).unwrap();
        let mut peer2 = MockCryptarchia::new();
        peer2.process_block(b1.clone()).unwrap();
        peer2.process_block(b4.clone()).unwrap();
        peer2.process_block(b5.clone()).unwrap();

        // Start a sync from genesis.
        //
        // Result: A merged block tree
        //                 b6 == tip
        //                /
        // G - b1 - b2 - b3
        //        \
        //          b4 - b5
        let mut local = MockCryptarchia::new();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer0), (1, &peer1), (2, &peer2)])),
        )
        .await
        .unwrap();
        assert_eq!(local.honest_chain, HeaderId::from([6; 32]));
        assert_eq!(local.forks, HashSet::from([HeaderId::from([5; 32])]));
        assert_eq!(local.blocks.len(), 7);
        assert_eq!(local.blocks_by_slot.len(), 5);
    }

    #[tokio::test]
    async fn reject_invalid_blocks() {
        // Prepare a peer with invalid blocks:
        // G - b1 - b2 - b3 - (invalid_b4) - (invalid_b5) == tip
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(i as u64),
                true,
            ))
            .unwrap();
        }
        for i in 4..=5 {
            peer.process_block_without_validation(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(i as u64),
                false,
            ))
            .unwrap();
        }
        assert_eq!(peer.honest_chain, HeaderId::from([5; 32]));
        assert!(peer.forks.is_empty());

        // Start a sync from genesis.
        // Result: The same honest chain, but without invalid blocks.
        // G - b1 - b2 - b3 == tip
        let mut local = MockCryptarchia::new();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local.honest_chain, HeaderId::from([3; 32]));
        assert_eq!(local.blocks.len(), 4);
        assert_eq!(local.blocks_by_slot.len(), 4);
    }

    #[tokio::test]
    async fn reject_invalid_blocks_from_backfilling() {
        // Prepare a peer with invalid blocks in a fork:
        // G - b1 - b2 - b3 - b7 - b8 == tip
        //        \
        //          b4 - (invalid_b5) - (invalid_b6)
        let mut peer = MockCryptarchia::new();
        peer.process_block(MockBlock::new(
            HeaderId::from([1; 32]),
            *GENESIS_ID,
            Slot::from(1),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([2; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([3; 32]),
            HeaderId::from([2; 32]),
            Slot::from(3),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([4; 32]),
            HeaderId::from([1; 32]),
            Slot::from(2),
            true,
        ))
        .unwrap();
        peer.process_block_without_validation(MockBlock::new(
            HeaderId::from([5; 32]),
            HeaderId::from([4; 32]),
            Slot::from(3),
            false,
        ))
        .unwrap();
        peer.process_block_without_validation(MockBlock::new(
            HeaderId::from([6; 32]),
            HeaderId::from([5; 32]),
            Slot::from(4),
            false,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([7; 32]),
            HeaderId::from([3; 32]),
            Slot::from(4),
            true,
        ))
        .unwrap();
        peer.process_block(MockBlock::new(
            HeaderId::from([8; 32]),
            HeaderId::from([7; 32]),
            Slot::from(5),
            true,
        ))
        .unwrap();
        assert_eq!(peer.honest_chain, HeaderId::from([8; 32]));
        assert_eq!(peer.forks, HashSet::from([HeaderId::from([6; 32])]));

        // Start a sync from a tree:
        // G - b1 - b3 - b4
        //
        // Result: The same forks, but without invalid blocks
        // G - b1 - b2 - b3 - b7 - b8 == tip
        //        \
        //          b4
        let mut local = MockCryptarchia::new();
        Synchronization::run(
            &mut local,
            &MockNetworkAdapter::new(HashMap::from([(0, &peer)])),
        )
        .await
        .unwrap();
        assert_eq!(local.honest_chain, HeaderId::from([8; 32]));
        assert_eq!(local.forks, HashSet::from([HeaderId::from([4; 32])]));
        assert_eq!(local.blocks.len(), 7);
        assert_eq!(local.blocks_by_slot.len(), 6);
    }

    static GENESIS_ID: LazyLock<HeaderId> = LazyLock::new(|| HeaderId::from([0; 32]));

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockBlock {
        id: HeaderId,
        parent: HeaderId,
        slot: Slot,
        is_valid: bool,
    }

    impl MockBlock {
        const fn new(id: HeaderId, parent: HeaderId, slot: Slot, is_valid: bool) -> Self {
            Self {
                id,
                parent,
                slot,
                is_valid,
            }
        }

        fn is_genesis(&self) -> bool {
            self.id == *GENESIS_ID
        }
    }

    impl AbstractBlock for MockBlock {
        fn id(&self) -> HeaderId {
            self.id
        }

        fn parent(&self) -> HeaderId {
            self.parent
        }

        fn slot(&self) -> Slot {
            self.slot
        }
    }

    /// Mock implementation of the Cryptarchia consensus algorithm,
    /// similar as the one in the executable specification.
    #[derive(Debug, PartialEq, Eq)]
    struct MockCryptarchia {
        blocks: HashMap<HeaderId, MockBlock>,
        blocks_by_slot: BTreeMap<Slot, HashSet<HeaderId>>,
        honest_chain: HeaderId,
        forks: HashSet<HeaderId>,
    }

    impl MockCryptarchia {
        fn new() -> Self {
            let genesis_block = MockBlock {
                id: *GENESIS_ID,
                parent: *GENESIS_ID,
                slot: Slot::from(0),
                is_valid: true,
            };
            let genesis_id = genesis_block.id();
            let genesis_slot = genesis_block.slot();
            Self {
                blocks: HashMap::from([(genesis_id, genesis_block)]),
                blocks_by_slot: BTreeMap::from([(genesis_slot, HashSet::from([genesis_id]))]),
                honest_chain: genesis_id,
                forks: HashSet::new(),
            }
        }

        fn process_block(&mut self, block: MockBlock) -> Result<(), CryptarchiaAdapterError> {
            if !block.is_valid {
                return Err(CryptarchiaAdapterError::InvalidBlock(
                    format!("Invalid block: {block:?}").into(),
                ));
            }
            self.process_block_without_validation(block)
        }

        fn process_block_without_validation(
            &mut self,
            block: MockBlock,
        ) -> Result<(), CryptarchiaAdapterError> {
            let id = block.id();
            let parent = block.parent();

            if self.blocks.contains_key(&id) {
                return Ok(());
            } else if !self.blocks.contains_key(&parent) {
                return Err(CryptarchiaAdapterError::ParentNotFound);
            }

            let slot = block.slot();
            self.blocks.insert(id, block);
            self.blocks_by_slot.entry(slot).or_default().insert(id);

            if parent == self.honest_chain {
                // simply extending the honest chain
                self.honest_chain = id;
            } else {
                // otherwise, this block creates a fork
                self.forks.insert(id);

                // remove any existing fork that is superceded by this block
                if self.forks.contains(&parent) {
                    self.forks.remove(&parent);
                }

                // We may need to switch forks, let's run the fork choice rule
                let new_honest_chain = self.fork_choice();
                self.forks.insert(self.honest_chain);
                self.forks.remove(&new_honest_chain);
                self.honest_chain = new_honest_chain;
            }

            Ok(())
        }

        /// Mock implementation of the fork choice rule.
        /// This always choose the block with the highest Id as the honest
        /// chain.
        fn fork_choice(&self) -> HeaderId {
            let mut honest_chain = self.honest_chain;
            for fork in &self.forks {
                honest_chain = honest_chain.max(*fork);
            }
            honest_chain
        }
    }

    #[async_trait::async_trait]
    impl CryptarchiaAdapter for MockCryptarchia {
        type Block = MockBlock;

        async fn process_block(
            &mut self,
            block: Self::Block,
        ) -> Result<(), CryptarchiaAdapterError> {
            self.process_block(block)
        }

        fn tip_slot(&self) -> Slot {
            self.blocks[&self.honest_chain].slot()
        }

        fn has_block(&self, id: &HeaderId) -> bool {
            self.blocks.contains_key(id)
        }
    }

    type PeerId = usize;

    struct MockNetworkAdapter<'a> {
        peers: HashMap<PeerId, &'a MockCryptarchia>,
    }

    impl<'a> MockNetworkAdapter<'a> {
        const fn new(peers: HashMap<PeerId, &'a MockCryptarchia>) -> Self {
            Self { peers }
        }
    }

    #[async_trait::async_trait]
    impl BlockFetcher for MockNetworkAdapter<'_> {
        type Block = MockBlock;
        type ProviderId = PeerId;

        async fn fetch_blocks_forward(
            &self,
            start_slot: Slot,
        ) -> Result<
            BoxedStream<(Self::Block, Self::ProviderId)>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            let mut blocks = Vec::new();
            for (peer_id, cryptarchia) in &self.peers {
                blocks.extend(
                    cryptarchia
                        .blocks_by_slot
                        .range(start_slot..)
                        .flat_map(|(_, ids)| ids)
                        .map(|id| cryptarchia.blocks.get(id).unwrap())
                        .cloned()
                        .map(|block| (block, *peer_id)),
                );
            }
            Ok(Box::new(futures::stream::iter(blocks)))
        }

        async fn fetch_chain_backward(
            &self,
            tip: HeaderId,
            provider_id: Self::ProviderId,
        ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>> {
            let mut blocks = Vec::new();
            let mut id = tip;
            let cryptarchia = self.peers.get(&provider_id).unwrap();
            while let Some(block) = cryptarchia.blocks.get(&id) {
                blocks.push(block.clone());
                if block.is_genesis() {
                    return Ok(Box::new(futures::stream::iter(blocks)));
                }
                id = block.parent;
            }
            Ok(Box::new(futures::stream::iter(blocks)))
        }
    }
}
