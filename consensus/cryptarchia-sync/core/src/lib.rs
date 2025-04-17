use std::error::Error;

use cryptarchia_engine::Slot;
use nomos_core::{block::AbstractBlock, header::HeaderId};

use crate::sync::CryptarchiaSyncExtInternal;

pub mod fetcher;
mod sync;

/// Defines Cryptarchia operations that are used during a chain sync process.
#[async_trait::async_trait]
pub trait CryptarchiaSync: Sized {
    type Block: AbstractBlock;

    /// Validate and apply a block to the block tree.
    async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaSyncError>;

    /// Get the slot of the tip block of the honest chain.
    fn tip_slot(&self) -> Slot;

    /// Check if the block is already in the block tree.
    fn has_block(&self, id: &HeaderId) -> bool;
}

/// Errors that should be handled in the sync process.
#[derive(thiserror::Error, Debug)]
pub enum CryptarchiaSyncError {
    #[error("Parent not found")]
    ParentNotFound,
    #[error("Invalid block: {0}")]
    InvalidBlock(Box<dyn Error + Send + Sync>),
}

/// Provides the actual chain sync initiation.
#[async_trait::async_trait]
pub trait CryptarchiaSyncExt: CryptarchiaSync + Sync + Send + Sized {
    async fn start_sync<BlockFetcher>(
        mut self,
        block_fetcher: &BlockFetcher,
    ) -> Result<Self, Box<dyn Error + Send + Sync>>
    where
        Self::Block: Send,
        BlockFetcher: fetcher::BlockFetcher<Block = Self::Block> + Sync,
    {
        self.run(block_fetcher).await
    }
}

impl<T: CryptarchiaSync + Sync + Send> CryptarchiaSyncExt for T {}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap, HashSet},
        num::NonZero,
        sync::LazyLock,
    };

    use cryptarchia_engine::{Branch, Slot};
    use futures::stream::BoxStream;
    use nomos_core::{block::AbstractBlock, header::HeaderId};

    use super::*;
    use crate::fetcher::BlockFetcher;

    #[tokio::test]
    async fn sync_single_chain_from_genesis() {
        // Prepare a peer with a single chain:
        // G - b1 - b2 - b3
        let mut peer = MockCryptarchia::new();
        for i in 1..=3 {
            peer.process_block(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(u64::from(i)),
                true,
            ))
            .unwrap();
        }
        assert_eq!(peer.engine.tip(), HeaderId::from([3; 32]));
        assert_eq!(peer.branch_tips(), HashSet::from([HeaderId::from([3; 32])]));

        // Start a sync from genesis.
        // Result: The same block tree as the peer's
        let local = MockCryptarchia::new()
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
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
                Slot::from(u64::from(i)),
                true,
            ))
            .unwrap();
        }
        assert_eq!(peer.engine.tip(), HeaderId::from([3; 32]));
        assert_eq!(peer.branch_tips(), HashSet::from([HeaderId::from([3; 32])]));

        // Start a sync from a tree:
        // G - b1
        //
        // Result: The same block tree as the peer's
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&HeaderId::from([1; 32])).unwrap().clone())
            .unwrap();
        let local = local
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
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
        assert_eq!(peer.engine.tip(), HeaderId::from([5; 32]));
        assert_eq!(
            peer.branch_tips(),
            HashSet::from([HeaderId::from([4; 32]), HeaderId::from([5; 32])])
        );

        // Start a sync from genesis.
        // Result: The same block tree as the peer's.
        let local = MockCryptarchia::new()
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
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
        assert_eq!(peer.engine.tip(), HeaderId::from([5; 32]));
        assert_eq!(
            peer.branch_tips(),
            HashSet::from([HeaderId::from([4; 32]), HeaderId::from([5; 32])])
        );

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
        let local = local
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
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
        assert_eq!(peer.engine.tip(), HeaderId::from([5; 32]));
        assert_eq!(
            peer.branch_tips(),
            HashSet::from([HeaderId::from([4; 32]), HeaderId::from([5; 32])])
        );

        // Start a sync from a tree without the fork:
        // G - b1
        //
        // Result: The same block tree as the peer's.
        let mut local = MockCryptarchia::new();
        local
            .process_block(peer.blocks.get(&HeaderId::from([1; 32])).unwrap().clone())
            .unwrap();
        let local = local
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
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
        let local = MockCryptarchia::new()
            .start_sync(&MockBlockFetcher::new(HashMap::from([
                (0, &peer0),
                (1, &peer1),
                (2, &peer2),
            ])))
            .await
            .unwrap();
        assert_eq!(local.engine.tip(), HeaderId::from([6; 32]));
        assert_eq!(
            local.branch_tips(),
            HashSet::from([HeaderId::from([5; 32]), HeaderId::from([6; 32])])
        );
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
                Slot::from(u64::from(i)),
                true,
            ))
            .unwrap();
        }
        for i in 4..=5 {
            peer.process_block_without_validation(MockBlock::new(
                HeaderId::from([i; 32]),
                HeaderId::from([i - 1; 32]),
                Slot::from(u64::from(i)),
                false,
            ))
            .unwrap();
        }
        assert_eq!(peer.engine.tip(), HeaderId::from([5; 32]));
        assert_eq!(peer.branch_tips(), HashSet::from([HeaderId::from([5; 32])]));

        // Start a sync from genesis.
        // Result: The same honest chain, but without invalid blocks.
        // G - b1 - b2 - b3 == tip
        let local = MockCryptarchia::new()
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
            .await
            .unwrap();
        assert_eq!(local.engine.tip(), HeaderId::from([3; 32]));
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
        assert_eq!(peer.engine.tip(), HeaderId::from([8; 32]));
        assert_eq!(
            peer.branch_tips(),
            HashSet::from([HeaderId::from([6; 32]), HeaderId::from([8; 32])])
        );

        // Start a sync from a tree:
        // G - b1 - b3 - b4
        //
        // Result: The same forks, but without invalid blocks
        // G - b1 - b2 - b3 - b7 - b8 == tip
        //        \
        //          b4
        let local = MockCryptarchia::new()
            .start_sync(&MockBlockFetcher::new(HashMap::from([(0, &peer)])))
            .await
            .unwrap();
        assert_eq!(local.engine.tip(), HeaderId::from([8; 32]));
        assert_eq!(
            local.branch_tips(),
            HashSet::from([HeaderId::from([4; 32]), HeaderId::from([8; 32])])
        );
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

    /// Mock implementation of the Cryptarchia consensus algorithm
    #[derive(Debug)]
    struct MockCryptarchia {
        // block storage
        blocks: HashMap<HeaderId, MockBlock>,
        blocks_by_slot: BTreeMap<Slot, HashSet<HeaderId>>,
        // cryptarchia engine
        engine: cryptarchia_engine::Cryptarchia<HeaderId>,
    }

    impl PartialEq for MockCryptarchia {
        fn eq(&self, other: &Self) -> bool {
            self.blocks == other.blocks
                && self.blocks_by_slot == other.blocks_by_slot
                && self.engine.genesis() == other.engine.genesis()
                && self.engine.tip() == other.engine.tip()
                && self.branches() == other.branches()
        }
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
                engine: cryptarchia_engine::Cryptarchia::new(
                    genesis_id,
                    cryptarchia_engine::Config {
                        security_param: NonZero::new(1000).unwrap(),
                        active_slot_coeff: 1.0,
                    },
                ),
            }
        }

        fn process_block(&mut self, block: MockBlock) -> Result<(), CryptarchiaSyncError> {
            if !block.is_valid {
                return Err(CryptarchiaSyncError::InvalidBlock(
                    format!("Invalid block: {block:?}").into(),
                ));
            }
            self.process_block_without_validation(block)
        }

        fn process_block_without_validation(
            &mut self,
            block: MockBlock,
        ) -> Result<(), CryptarchiaSyncError> {
            if self.blocks.contains_key(&block.id()) {
                return Ok(());
            }

            match self
                .engine
                .receive_block(block.id(), block.parent(), block.slot())
            {
                Ok(engine) => {
                    self.engine = engine;
                    self.blocks_by_slot
                        .entry(block.slot())
                        .or_default()
                        .insert(block.id());
                    self.blocks.insert(block.id(), block);
                    Ok(())
                }
                Err(cryptarchia_engine::Error::ParentMissing(_)) => {
                    Err(CryptarchiaSyncError::ParentNotFound)
                }
                Err(e) => Err(CryptarchiaSyncError::InvalidBlock(e.into())),
            }
        }

        fn branches(&self) -> HashMap<HeaderId, Branch<HeaderId>> {
            self.engine
                .branches()
                .branches()
                .iter()
                .cloned()
                .map(|branch| (branch.id(), branch))
                .collect()
        }

        fn branch_tips(&self) -> HashSet<HeaderId> {
            self.engine
                .branches()
                .branches()
                .iter()
                .map(Branch::id)
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl CryptarchiaSync for MockCryptarchia {
        type Block = MockBlock;

        async fn process_block(&mut self, block: Self::Block) -> Result<(), CryptarchiaSyncError> {
            self.process_block(block)
        }

        fn tip_slot(&self) -> Slot {
            self.engine
                .branches()
                .get(&self.engine.tip())
                .unwrap()
                .slot()
        }

        fn has_block(&self, id: &HeaderId) -> bool {
            self.blocks.contains_key(id)
        }
    }

    type PeerId = usize;

    struct MockBlockFetcher<'a> {
        peers: HashMap<PeerId, &'a MockCryptarchia>,
    }

    impl<'a> MockBlockFetcher<'a> {
        const fn new(peers: HashMap<PeerId, &'a MockCryptarchia>) -> Self {
            Self { peers }
        }
    }

    #[async_trait::async_trait]
    impl BlockFetcher for MockBlockFetcher<'_> {
        type Block = MockBlock;
        type ProviderId = PeerId;

        async fn fetch_blocks_forward(
            &self,
            start_slot: Slot,
        ) -> Result<
            BoxStream<(Self::Block, Self::ProviderId)>,
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
            Ok(Box::pin(futures::stream::iter(blocks)))
        }

        async fn fetch_chain_backward(
            &self,
            tip: HeaderId,
            provider_id: Self::ProviderId,
        ) -> Result<BoxStream<Self::Block>, Box<dyn std::error::Error + Send + Sync>> {
            let mut blocks = Vec::new();
            let mut id = tip;
            let cryptarchia = self.peers.get(&provider_id).unwrap();
            while let Some(block) = cryptarchia.blocks.get(&id) {
                blocks.push(block.clone());
                if block.is_genesis() {
                    return Ok(Box::pin(futures::stream::iter(blocks)));
                }
                id = block.parent;
            }
            Ok(Box::pin(futures::stream::iter(blocks)))
        }
    }
}
