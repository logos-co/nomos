pub mod adapter;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    marker::{PhantomData, Unpin},
};

use adapter::{CryptarchiaAdapter, CryptarchiaAdapterError, NetworkAdapter};
use futures::{Stream, StreamExt};
use itertools::Itertools;
use nomos_core::block::AbstractBlock;
use tracing::info;

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
            let mut num_processed_blocks = 0;

            // TODO: handle network error
            info!("Fetching blocks from slot {:?}", cryptarchia.tip_slot() + 1);
            let mut stream = network
                .fetch_blocks_from_slot(cryptarchia.tip_slot() + 1)
                .await?;

            while let Some(block) = stream.next().await {
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
                        num_processed_blocks += 1;
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

            // Finish the sync process if no block has been processed,
            // which means that no peer has blocks that the local node doesn't know.
            if num_processed_blocks == 0 {
                info!("No new blocks to process");
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cryptarchia_engine::Slot;
    use nomos_core::block::AbstractBlock;

    use super::*;
    use crate::adapter::{BoxedStream, CryptarchiaAdapter, CryptarchiaAdapterError};

    #[tokio::test]
    async fn sync_single_chain_from_genesis() {
        // Prepare a peer with a single chain:
        // G - b1 - b2 - b3
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(i, i - 1, Slot::from(i), true))
                .unwrap();
        }
        assert_eq!(peer.honest_chain, 3);
        assert!(peer.forks.is_empty());

        // Start a sync from genesis.
        // Result: The same block tree as the peer's
        let local = MockCryptarchia::new();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
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
            peer.process_block(MockBlock::new(i, i - 1, Slot::from(i), true))
                .unwrap();
        }
        assert_eq!(peer.honest_chain, 3);
        assert!(peer.forks.is_empty());

        // Start a sync from a tree:
        // G - b1
        //
        // Result: The same block tree as the peer's
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&1).unwrap().clone())
            .unwrap();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
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
        peer.process_block(MockBlock::new(1, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(2, 1, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(3, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(4, 3, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(5, 2, Slot::from(3), true))
            .unwrap();
        assert_eq!(peer.honest_chain, 5);
        assert_eq!(peer.forks, HashSet::from([4]));

        // Start a sync from genesis.
        // Result: The same block tree as the peer's.
        let local = MockCryptarchia::new();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
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
        peer.process_block(MockBlock::new(1, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(2, 1, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(3, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(4, 3, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(5, 2, Slot::from(3), true))
            .unwrap();
        assert_eq!(peer.honest_chain, 5);
        assert_eq!(peer.forks, HashSet::from([4]));

        // Start a sync from a tree:
        // G - b1
        //   \
        //     b3
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&1).unwrap().clone())
            .unwrap();
        local
            .process_block(peer.blocks.get(&3).unwrap().clone())
            .unwrap();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
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
        peer.process_block(MockBlock::new(1, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(2, 1, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(3, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(4, 3, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(5, 2, Slot::from(3), true))
            .unwrap();
        assert_eq!(peer.honest_chain, 5);
        assert_eq!(peer.forks, HashSet::from([4]));

        // Start a sync from a tree without the fork:
        // G - b1
        //
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&1).unwrap().clone())
            .unwrap();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
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
        let b1 = MockBlock::new(1, GENESIS_ID, Slot::from(1), true);
        let b2 = MockBlock::new(2, 1, Slot::from(2), true);
        let b3 = MockBlock::new(3, 2, Slot::from(3), true);
        let b4 = MockBlock::new(4, 1, Slot::from(2), true);
        let b5 = MockBlock::new(5, 4, Slot::from(3), true);
        let b6 = MockBlock::new(6, 3, Slot::from(4), true);
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
        let local = MockCryptarchia::new();
        let local = Synchronization::run(
            local,
            &MockNetworkAdapter::new(vec![&peer0, &peer1, &peer2]),
        )
        .await
        .unwrap();
        assert_eq!(local.honest_chain, 6);
        assert_eq!(local.forks, HashSet::from([5]));
        assert_eq!(local.blocks.len(), 7);
        assert_eq!(local.blocks_by_slot.len(), 5);
    }

    #[tokio::test]
    async fn reject_invalid_blocks() {
        // Prepare a peer with invalid blocks:
        // G - b1 - b2 - b3 - (invalid_b4) - (invalid_b5) == tip
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(i, i - 1, Slot::from(i), true))
                .unwrap();
        }
        for i in 4..=5 {
            peer.process_block_without_validation(MockBlock::new(i, i - 1, Slot::from(i), false))
                .unwrap();
        }
        assert_eq!(peer.honest_chain, 5);
        assert!(peer.forks.is_empty());

        // Start a sync from genesis.
        // Result: The same honest chain, but without invalid blocks.
        // G - b1 - b2 - b3 == tip
        let local = MockCryptarchia::new();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
            .await
            .unwrap();
        assert_eq!(local.honest_chain, 3);
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
        peer.process_block(MockBlock::new(1, GENESIS_ID, Slot::from(1), true))
            .unwrap();
        peer.process_block(MockBlock::new(2, 1, Slot::from(2), true))
            .unwrap();
        peer.process_block(MockBlock::new(3, 2, Slot::from(3), true))
            .unwrap();
        peer.process_block(MockBlock::new(4, 1, Slot::from(2), true))
            .unwrap();
        peer.process_block_without_validation(MockBlock::new(5, 4, Slot::from(3), false))
            .unwrap();
        peer.process_block_without_validation(MockBlock::new(6, 5, Slot::from(4), false))
            .unwrap();
        peer.process_block(MockBlock::new(7, 3, Slot::from(4), true))
            .unwrap();
        peer.process_block(MockBlock::new(8, 7, Slot::from(5), true))
            .unwrap();
        assert_eq!(peer.honest_chain, 8);
        assert_eq!(peer.forks, HashSet::from([6]));

        // Start a sync from a tree:
        // G - b1 - b3 - b4
        //
        // Result: The same forks, but without invalid blocks
        // G - b1 - b2 - b3 - b7 - b8 == tip
        //        \
        //          b4
        let local = MockCryptarchia::new();
        let local = Synchronization::run(local, &MockNetworkAdapter::new(vec![&peer]))
            .await
            .unwrap();
        assert_eq!(local.honest_chain, 8);
        assert_eq!(local.forks, HashSet::from([4]));
        assert_eq!(local.blocks.len(), 7);
        assert_eq!(local.blocks_by_slot.len(), 6);
    }

    type Id = u64;
    const GENESIS_ID: Id = 0;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockBlock {
        id: Id,
        parent: Id,
        slot: Slot,
        is_valid: bool,
    }

    impl MockBlock {
        const fn new(id: Id, parent: Id, slot: Slot, is_valid: bool) -> Self {
            Self {
                id,
                parent,
                slot,
                is_valid,
            }
        }

        const fn is_genesis(&self) -> bool {
            self.id == GENESIS_ID
        }
    }

    impl AbstractBlock for MockBlock {
        type Id = u64;

        fn id(&self) -> Self::Id {
            self.id
        }

        fn parent(&self) -> Self::Id {
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
        blocks: HashMap<Id, MockBlock>,
        blocks_by_slot: BTreeMap<Slot, HashSet<Id>>,
        honest_chain: Id,
        forks: HashSet<Id>,
    }

    impl MockCryptarchia {
        fn new() -> Self {
            let genesis_block = MockBlock {
                id: GENESIS_ID,
                parent: GENESIS_ID,
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
        fn fork_choice(&self) -> Id {
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

        fn has_block(&self, id: &<Self::Block as AbstractBlock>::Id) -> bool {
            self.blocks.contains_key(id)
        }
    }

    struct MockNetworkAdapter<'a> {
        peers: Vec<&'a MockCryptarchia>,
    }

    impl<'a> MockNetworkAdapter<'a> {
        const fn new(peers: Vec<&'a MockCryptarchia>) -> Self {
            Self { peers }
        }
    }

    #[async_trait::async_trait]
    impl NetworkAdapter for MockNetworkAdapter<'_> {
        type Block = MockBlock;

        async fn fetch_blocks_from_slot(
            &self,
            start_slot: Slot,
        ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>> {
            let mut blocks = Vec::new();
            for peer in &self.peers {
                blocks.extend(
                    peer.blocks_by_slot
                        .range(start_slot..)
                        .flat_map(|(_, ids)| ids)
                        .map(|id| peer.blocks.get(id).unwrap())
                        .cloned(),
                );
            }
            Ok(Box::new(futures::stream::iter(blocks)))
        }

        async fn fetch_chain_backward(
            &self,
            tip: <Self::Block as AbstractBlock>::Id,
        ) -> Result<BoxedStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>> {
            let mut blocks = Vec::new();
            let mut id = tip;
            for peer in &self.peers {
                while let Some(block) = peer.blocks.get(&id) {
                    blocks.push(block.clone());
                    if block.is_genesis() {
                        return Ok(Box::new(futures::stream::iter(blocks)));
                    }
                    id = block.parent;
                }
            }
            Ok(Box::new(futures::stream::iter(blocks)))
        }
    }
}
