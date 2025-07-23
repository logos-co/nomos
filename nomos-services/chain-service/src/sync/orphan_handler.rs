use std::collections::{HashSet, VecDeque};

use futures::StreamExt as _;
use nomos_core::{block::Block, header::HeaderId};
use overwatch::DynError;
use tracing::{error, info};

use crate::network::{BoxedStream, NetworkAdapter};

const MAX_ORPHAN_CACHE_SIZE: usize = 50;

/// Handles orphan blocks by fetching their ancestors
pub struct OrphanHandler<NetAdapter, Tx, BlockCert, RuntimeServiceId>
where
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
{
    /// Queue of orphan IDs waiting to be processed
    pending_orphans: VecDeque<HeaderId>,
    /// Currently active network stream (if any)
    current_stream: Option<BoxedStream<Result<Block<Tx, BlockCert>, DynError>>>,
    /// Network adapter for making requests
    network_adapter: NetAdapter,
    /// Current orphan processing state
    current_orphan_state: Option<OrphanHandlingState<Tx, BlockCert>>,
    _phantom: std::marker::PhantomData<RuntimeServiceId>,
}

/// Tracks the state of current orphan processing
struct OrphanHandlingState<Tx, BlockCert>
where
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
{
    /// The orphan block we're trying to resolve
    target: HeaderId,
    /// Last block received (used to continue fetching)
    last_block: Option<HeaderId>,
    /// Active stream for this orphan (None between batches)
    stream: Option<BoxedStream<Result<Block<Tx, BlockCert>, DynError>>>,
}

impl<NetAdapter, Tx, BlockCert, RuntimeServiceId>
    OrphanHandler<NetAdapter, Tx, BlockCert, RuntimeServiceId>
where
    NetAdapter:
        NetworkAdapter<RuntimeServiceId, Block = Block<Tx, BlockCert>> + Send + Sync + Clone,
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
    RuntimeServiceId: Send + Sync + 'static,
{
    pub fn new(network_adapter: NetAdapter) -> Self {
        Self {
            pending_orphans: VecDeque::new(),
            current_stream: None,
            network_adapter,
            current_orphan_state: None,
            _phantom: std::marker::PhantomData,
        }
    }

    pub async fn request_blocks(
        &mut self,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
    ) -> Option<Block<Tx, BlockCert>> {
        if let Some(block) = self.poll_stream().await {
            return Some(block);
        }

        self.start_orphan_stream(local_tip, latest_immutable_block)
            .await;

        None
    }

    async fn start_orphan_stream(&mut self, local_tip: HeaderId, latest_immutable_block: HeaderId) {
        let (target, last_block) = match &self.current_orphan_state {
            Some(state) if state.last_block.is_some() => (state.target, state.last_block),
            _ => {
                self.current_orphan_state = None;
                if let Some(orphan_id) = self.pending_orphans.pop_front() {
                    (orphan_id, None)
                } else {
                    return;
                }
            }
        };

        let mut additional_blocks = HashSet::new();
        if let Some(block) = last_block {
            additional_blocks.insert(block);
        }

        match self
            .network_adapter
            .request_blocks_from_peers(target, local_tip, latest_immutable_block, additional_blocks)
            .await
        {
            Ok(stream) => {
                self.current_orphan_state = Some(OrphanHandlingState {
                    target,
                    last_block: None,
                    stream: Some(stream),
                });
            }
            Err(e) => {
                error!("Failed to request orphan stream for {target:?}: {e}");
                self.current_orphan_state = None;
            }
        }
    }

    async fn poll_stream(&mut self) -> Option<Block<Tx, BlockCert>> {
        let state = self.current_orphan_state.as_mut()?;
        let stream = state.stream.as_mut()?;

        match stream.next().await {
            Some(Ok(block)) => {
                state.last_block = Some(block.header().id());
                Some(block)
            }
            Some(Err(e)) => {
                error!("Stream error: {}", e);
                self.current_orphan_state = None;
                None
            }
            None => {
                state.stream = None;

                if state.last_block.is_none() {
                    self.current_orphan_state = None;
                }
                None
            }
        }
    }

    pub fn add_orphan(&mut self, orphan_id: HeaderId) {
        if self.pending_orphans.len() >= MAX_ORPHAN_CACHE_SIZE {
            return;
        }

        if let Some(ref state) = self.current_orphan_state {
            if state.target == orphan_id {
                return;
            }
        }

        if !self.pending_orphans.contains(&orphan_id) {
            self.pending_orphans.push_back(orphan_id);
        }
    }

    pub fn clear_orphan(&mut self, block_id: &HeaderId) {
        self.pending_orphans.retain(|id| id != block_id);

        if let Some(ref state) = self.current_orphan_state {
            if &state.target == block_id {
                self.cancel_active_stream();
            }
        }
    }

    pub fn cancel_active_stream(&mut self) {
        if let Some(state) = self.current_orphan_state.take() {
            info!("Cancelling orphan handling for {:?}", state.target);
        }
    }

    pub fn should_call(&self) -> bool {
        !self.pending_orphans.is_empty() || self.current_orphan_state.is_some()
    }

    fn is_streaming(&self) -> bool {
        self.current_orphan_state
            .as_ref()
            .and_then(|state| state.stream.as_ref())
            .is_some()
    }

    fn current_orphan(&self) -> Option<&HeaderId> {
        self.current_orphan_state.as_ref().map(|s| &s.target)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cryptarchia_engine::Slot;
    use cryptarchia_sync::GetTipResponse;
    use futures::stream;
    use kzgrs_backend::dispersal::BlobInfo;
    use nomos_core::{
        block::builder::BlockBuilder,
        da::blob::select::FillSize as BlobFillSize,
        header::Builder,
        mantle::{select::FillSize as TxFillSize, SignedMantleTx},
        proofs::leader_proof::Risc0LeaderProof,
    };
    use nomos_network::{backends::mock::Mock, message::ChainSyncEvent, NetworkService};
    use nomos_proof_statements::leadership::{LeaderPrivate, LeaderPublic};
    use overwatch::services::{relay::OutboundRelay, ServiceData};
    use tokio::time::{timeout, Duration};

    use super::*;

    type OrphanResults = Vec<Result<Block<SignedMantleTx, BlobInfo>, String>>;

    async fn poll_until_block_or_done<NetAdapter>(
        handler: &mut OrphanHandler<NetAdapter, SignedMantleTx, BlobInfo, usize>,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
    ) -> Option<Block<SignedMantleTx, BlobInfo>>
    where
        NetAdapter:
            NetworkAdapter<usize, Block = Block<SignedMantleTx, BlobInfo>> + Send + Sync + Clone,
    {
        timeout(Duration::from_secs(1), async {
            loop {
                match handler
                    .request_blocks(local_tip, latest_immutable_block)
                    .await
                {
                    Some(block) => return Some(block),
                    _ => {
                        if !handler.should_call() {
                            return None;
                        }
                    }
                }
            }
        })
        .await
        .expect("Test timed out waiting for block")
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct BlockRequest {
        target: HeaderId,
        local_tip: HeaderId,
        lib: HeaderId,
        additional_blocks: HashSet<HeaderId>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct ResponseKey {
        orphan: HeaderId,
        continuation_from: Option<HeaderId>,
    }

    #[derive(Clone)]
    struct MockNetworkAdapter {
        requests: Vec<BlockRequest>,
        responses: HashMap<ResponseKey, OrphanResults>,
    }

    impl MockNetworkAdapter {
        fn new() -> Self {
            Self {
                requests: Vec::new(),
                responses: HashMap::new(),
            }
        }

        fn with_responses(
            mut self,
            orphan_id: HeaderId,
            blocks: Vec<Block<SignedMantleTx, BlobInfo>>,
        ) -> Self {
            let key = ResponseKey {
                orphan: orphan_id,
                continuation_from: None,
            };
            self.responses
                .insert(key, blocks.into_iter().map(Ok).collect());
            self
        }

        fn with_continuation(
            mut self,
            orphan_id: HeaderId,
            last_block: HeaderId,
            blocks: Vec<Block<SignedMantleTx, BlobInfo>>,
        ) -> Self {
            let key = ResponseKey {
                orphan: orphan_id,
                continuation_from: Some(last_block),
            };
            self.responses
                .insert(key, blocks.into_iter().map(Ok).collect());
            self
        }

        fn with_error(mut self, orphan_id: HeaderId, error: &str) -> Self {
            let key = ResponseKey {
                orphan: orphan_id,
                continuation_from: None,
            };
            self.responses.insert(key, vec![Err(error.into())]);
            self
        }
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId: Send + Sync> NetworkAdapter<RuntimeServiceId> for MockNetworkAdapter {
        type Backend = Mock;
        type Settings = ();
        type PeerId = ();
        type Block = Block<SignedMantleTx, BlobInfo>;

        async fn new(
            _settings: Self::Settings,
            _network_relay: OutboundRelay<
                <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
            >,
        ) -> Self {
            Self::new()
        }

        async fn blocks_stream(&self) -> Result<BoxedStream<Self::Block>, DynError> {
            unimplemented!()
        }

        async fn chainsync_events_stream(&self) -> Result<BoxedStream<ChainSyncEvent>, DynError> {
            unimplemented!()
        }

        async fn request_tip(&self, _peer: Self::PeerId) -> Result<GetTipResponse, DynError> {
            unimplemented!()
        }

        async fn request_blocks_from_peer(
            &self,
            _peer: Self::PeerId,
            _target_block: HeaderId,
            _local_tip: HeaderId,
            _latest_immutable_block: HeaderId,
            _additional_blocks: HashSet<HeaderId>,
        ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError> {
            unimplemented!()
        }

        async fn request_blocks_from_peers(
            &mut self,
            target_block: HeaderId,
            local_tip: HeaderId,
            latest_immutable_block: HeaderId,
            additional_blocks: HashSet<HeaderId>,
        ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError> {
            let request = BlockRequest {
                target: target_block,
                local_tip,
                lib: latest_immutable_block,
                additional_blocks: additional_blocks.clone(),
            };
            self.requests.push(request);

            let key = ResponseKey {
                orphan: target_block,
                continuation_from: additional_blocks.iter().next().copied(),
            };

            self.responses.remove(&key).map_or_else(
                || Err("No response configured for this request".into()),
                |blocks| {
                    let stream = stream::iter(blocks)
                        .map(|result| result.map_err(Into::into))
                        .boxed();
                    Ok(Box::new(stream) as BoxedStream<Result<Self::Block, DynError>>)
                },
            )
        }
    }

    // Replace it later with Block trait
    fn create_block(parent: HeaderId, proof: &Risc0LeaderProof) -> Block<SignedMantleTx, BlobInfo> {
        BlockBuilder::new(
            TxFillSize::<1, SignedMantleTx>::new(),
            BlobFillSize::<1, BlobInfo>::new(),
            Builder::new(parent, Slot::from(0), proof.clone()),
        )
        .with_transactions(vec![].into_iter())
        .with_blobs_info(vec![].into_iter())
        .build()
        .expect("Block should be built successfully")
    }

    fn make_test_proof() -> Risc0LeaderProof {
        let public_inputs = LeaderPublic::new(
            [1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32], 0u64, 0.05f64, 1000u64,
        );
        let private_inputs = LeaderPrivate {
            value: 100,
            note_id: [5u8; 32],
            sk: [6u8; 16],
        };
        Risc0LeaderProof::prove(
            public_inputs,
            &private_inputs,
            risc0_zkvm::default_prover().as_ref(),
        )
        .expect("Proof generation should succeed")
    }

    #[tokio::test]
    async fn test_orphan_cache_limit() {
        let network = MockNetworkAdapter::new();
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        for i in 0..MAX_ORPHAN_CACHE_SIZE {
            handler.add_orphan([i as u8; 32].into());
        }

        assert_eq!(handler.pending_orphans.len(), MAX_ORPHAN_CACHE_SIZE);

        let extra_orphan = [255u8; 32].into();
        handler.add_orphan(extra_orphan);

        assert_eq!(handler.pending_orphans.len(), MAX_ORPHAN_CACHE_SIZE);
        assert!(!handler.pending_orphans.contains(&extra_orphan));
    }

    #[tokio::test]
    async fn test_remove_orphan() {
        let network = MockNetworkAdapter::new();
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        let orphan_id = [1u8; 32].into();
        handler.add_orphan(orphan_id);

        assert!(handler.should_call());

        handler.clear_orphan(&orphan_id);

        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_has_orphans_with_active_stream() {
        let proof = make_test_proof();

        let parent_block = create_block([0u8; 32].into(), &proof);
        let orphan_block = create_block(parent_block.header().id(), &proof);

        let orphan_id = orphan_block.header().id();

        let blocks = vec![parent_block, orphan_block];

        let network = MockNetworkAdapter::new().with_responses(orphan_id, blocks);
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan_id);

        let _ = handler
            .request_blocks([10u8; 32].into(), [11u8; 32].into())
            .await;

        assert_eq!(handler.pending_orphans.len(), 0);
        assert!(handler.should_call());
    }

    #[tokio::test]
    async fn test_next_block_successful_stream() {
        let proof = make_test_proof();

        let block1 = create_block([0u8; 32].into(), &proof);
        let block2 = create_block(block1.header().id(), &proof);
        let orphan_block = create_block(block2.header().id(), &proof);
        let orphan_id = orphan_block.header().id();

        let blocks = vec![block1.clone(), block2.clone(), orphan_block.clone()];
        let expected_ids: Vec<_> = blocks.iter().map(|b| b.header().id()).collect();

        let network = MockNetworkAdapter::new()
            .with_responses(orphan_id, blocks)
            .with_continuation(orphan_id, orphan_block.header().id(), vec![]);

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan_id);

        for expected_id in &expected_ids {
            let block =
                poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

            assert_eq!(block.as_ref().map(|b| b.header().id()), Some(*expected_id));
        }

        // Make sure we get None when no more blocks are available
        let block =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(block.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_next_block_network_error() {
        let orphan_id = [1u8; 32].into();
        let network = MockNetworkAdapter::new().with_error(orphan_id, "Network timeout");
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan_id);

        let block =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(block.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_multiple_orphans() {
        let proof = make_test_proof();

        let chain1_block = create_block([0u8; 32].into(), &proof);
        let orphan1 = create_block(chain1_block.header().id(), &proof);
        let orphan1_id = orphan1.header().id();

        let chain2_block = create_block([1u8; 32].into(), &proof);
        let orphan2 = create_block(chain2_block.header().id(), &proof);
        let orphan2_id = orphan2.header().id();

        let blocks1 = vec![chain1_block.clone()];
        let blocks2 = vec![chain2_block.clone()];

        let network = MockNetworkAdapter::new()
            .with_responses(orphan1_id, blocks1)
            .with_responses(orphan2_id, blocks2)
            .with_continuation(orphan1_id, chain1_block.header().id(), vec![])
            .with_continuation(orphan2_id, chain2_block.header().id(), vec![]);

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let block1 =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert_eq!(
            block1.map(|b| b.header().id()),
            Some(chain1_block.header().id())
        );

        let block2 =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert_eq!(
            block2.map(|b| b.header().id()),
            Some(chain2_block.header().id())
        );

        let none =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(none.is_none());

        // Check requests - expecting 4 (2 initial + 2 continuation)
        let requests = handler.network_adapter.requests;
        assert_eq!(requests.len(), 4);

        assert_eq!(requests[0].additional_blocks.len(), 0);
        assert_eq!(requests[1].additional_blocks.len(), 1);
        assert_eq!(requests[2].additional_blocks.len(), 0);
        assert_eq!(requests[3].additional_blocks.len(), 1);
    }

    #[tokio::test]
    async fn test_stream_with_errors_midway() {
        let proof = make_test_proof();

        let block1 = create_block([0u8; 32].into(), &proof);
        let block2 = create_block(block1.header().id(), &proof);
        let orphan = create_block(block2.header().id(), &proof);
        let orphan_id = orphan.header().id();

        let mut network = MockNetworkAdapter::new();
        network.responses.insert(
            ResponseKey {
                orphan: orphan_id,
                continuation_from: None,
            },
            vec![Ok(block1), Err("Stream error".into()), Ok(block2)],
        );

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);
        handler.add_orphan(orphan_id);

        let block1 =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(block1.is_some());

        let block2 =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(block2.is_none());

        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_cancel_active_stream() {
        let proof = make_test_proof();

        let block1 = create_block([0u8; 32].into(), &proof);
        let block2 = create_block(block1.header().id(), &proof);
        let block3 = create_block(block2.header().id(), &proof);
        let orphan_id = block3.header().id();

        let blocks = vec![block1.clone(), block2.clone(), block3.clone()];

        let network = MockNetworkAdapter::new().with_responses(orphan_id, blocks);
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan_id);

        let first =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(first.is_some());
        assert!(handler.is_streaming());

        handler.cancel_active_stream();
        assert!(!handler.is_streaming());

        let next =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(next.is_none());
    }

    #[tokio::test]
    async fn test_cancel_stream_continues_with_next_orphan() {
        let proof = make_test_proof();

        let chain1_block1 = create_block([0u8; 32].into(), &proof);
        let chain1_block2 = create_block(chain1_block1.header().id(), &proof);
        let orphan1_id = chain1_block2.header().id();

        let chain2_block1 = create_block([1u8; 32].into(), &proof);
        let chain2_block2 = create_block(chain2_block1.header().id(), &proof);
        let orphan2_id = chain2_block2.header().id();

        let blocks1 = vec![chain1_block1.clone(), chain1_block2.clone()];
        let blocks2 = vec![chain2_block1.clone(), chain2_block2.clone()];

        let network = MockNetworkAdapter::new()
            .with_responses(orphan1_id, blocks1)
            .with_responses(orphan2_id, blocks2);

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let block =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert_eq!(
            block.map(|b| b.header().id()),
            Some(chain1_block1.header().id())
        );

        handler.cancel_active_stream();

        let block =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert_eq!(
            block.map(|b| b.header().id()),
            Some(chain2_block1.header().id())
        );
    }

    #[tokio::test]
    async fn test_remove_orphan_during_streaming() {
        let proof = make_test_proof();

        let block1 = create_block([0u8; 32].into(), &proof);
        let block2 = create_block(block1.header().id(), &proof);
        let orphan1 = create_block(block2.header().id(), &proof);
        let orphan1_id = orphan1.header().id();

        let orphan2_id = [99u8; 32].into();

        let blocks = vec![block1.clone(), block2.clone(), orphan1.clone()];

        let network = MockNetworkAdapter::new()
            .with_responses(orphan1_id, blocks)
            .with_continuation(orphan1_id, orphan1.header().id(), vec![]);

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let first =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(first.is_some());

        handler.clear_orphan(&orphan2_id);

        let second =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(second.is_some());

        let third =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(third.is_some());

        let done =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(done.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_is_streaming() {
        let proof = make_test_proof();

        let block = create_block([0u8; 32].into(), &proof);
        let orphan = create_block(block.header().id(), &proof);
        let orphan_id = orphan.header().id();

        let blocks = vec![block, orphan];

        let network = MockNetworkAdapter::new().with_responses(orphan_id, blocks);
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        assert!(!handler.is_streaming());

        handler.add_orphan(orphan_id);

        let _ = handler
            .request_blocks([10u8; 32].into(), [11u8; 32].into())
            .await;
        assert!(handler.is_streaming());

        let _ = poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;
        // May or may not be streaming depending on if we got all blocks
    }

    #[tokio::test]
    async fn test_network_error_clears_stream_state() {
        let orphan_id = [1u8; 32].into();
        let network = MockNetworkAdapter::new().with_error(orphan_id, "Network timeout");
        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);

        handler.add_orphan(orphan_id);
        assert!(!handler.is_streaming());

        let block =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;

        assert!(block.is_none());
        assert!(!handler.is_streaming());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_continuation_requests() {
        let proof = make_test_proof();

        // Create a chain of 3 blocks
        let block0 = create_block([0u8; 32].into(), &proof);
        let block1 = create_block(block0.header().id(), &proof);
        let block2 = create_block(block1.header().id(), &proof);

        let blocks = vec![block0.clone(), block1.clone(), block2.clone()];
        let orphan_id = block2.header().id();

        // Set up mock to return blocks and then empty continuation
        let network = MockNetworkAdapter::new()
            .with_responses(orphan_id, blocks.clone())
            .with_continuation(orphan_id, block2.header().id(), vec![]);

        let mut handler: OrphanHandler<_, _, _, usize> = OrphanHandler::new(network);
        handler.add_orphan(orphan_id);

        // Get all 3 blocks
        for expected_block in &blocks {
            let received =
                poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;
            assert_eq!(
                received.map(|b| b.header().id()),
                Some(expected_block.header().id())
            );
        }

        // Next call should return None (continuation returned empty)
        let none =
            poll_until_block_or_done(&mut handler, [10u8; 32].into(), [11u8; 32].into()).await;
        assert!(none.is_none());

        // Should have made 2 requests total
        let requests = handler.network_adapter.requests;
        assert_eq!(requests.len(), 2); // Initial + continuation

        // Check first request has no additional blocks
        assert_eq!(requests[0].additional_blocks.len(), 0);

        // Check continuation request has the last block
        assert_eq!(requests[1].additional_blocks.len(), 1);
        assert!(requests[1]
            .additional_blocks
            .contains(&block2.header().id()));
    }
}
