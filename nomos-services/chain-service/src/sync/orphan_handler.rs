use std::{
    collections::HashSet,
    num::{NonZero, NonZeroUsize},
};

use futures::StreamExt as _;
use nomos_core::{block::Block, header::HeaderId};
use overwatch::DynError;
use tracing::{debug, error};

use crate::{
    network::{BoxedStream, NetworkAdapter},
    sync::block_provider::MAX_NUMBER_OF_BLOCKS,
};

/// Handles orphan blocks by fetching their ancestors
pub struct OrphanHandler<NetAdapter, Tx, BlockCert, RuntimeServiceId>
where
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
{
    /// Queue of orphan IDs waiting to be processed
    pending_orphans: HashSet<HeaderId>,
    /// Network adapter for making requests
    network_adapter: NetAdapter,
    /// Current orphan processing state
    current_orphan_state: Option<OrphanHandlingState<Tx, BlockCert>>,
    /// Maximum size of the orphan cache
    max_orphan_cache_size: NonZeroUsize,
    /// Maximum number of download requests per orphan
    max_number_of_iterations: NonZeroUsize,
    _phantom: std::marker::PhantomData<RuntimeServiceId>,
}

/// Tracks the state of current orphan processing
struct OrphanHandlingState<Tx, BlockCert>
where
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
{
    /// The orphan block we're trying to handle
    orphan_id: HeaderId,
    /// Last block received (used to continue fetching)
    last_block: Option<HeaderId>,
    /// Active stream for this orphan (None between batches)
    stream: Option<BoxedStream<Result<Block<Tx, BlockCert>, DynError>>>,
    /// Number of requests made for this orphan
    requests_count: usize,
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
    pub fn new(
        network_adapter: NetAdapter,
        max_orphan_cache_size: NonZeroUsize,
        security_param: NonZero<u32>,
    ) -> Self {
        // We need to fetch at most security_param blocks.
        let blocks_to_fetch = security_param.get() as usize;
        let iterations = blocks_to_fetch.div_ceil(MAX_NUMBER_OF_BLOCKS);
        let max_number_of_iterations =
            NonZeroUsize::new(iterations).expect("NonZeroUsize should be non-zero");

        Self {
            pending_orphans: HashSet::default(),
            network_adapter,
            current_orphan_state: None,
            max_orphan_cache_size,
            max_number_of_iterations,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Either returns a block from the current orphan stream or starts a new
    /// orphan stream. When starting a new stream, the first block will be
    /// returned on the next call.
    ///
    /// @param `local_tip` Current local tip of the chain.
    /// @param `latest_immutable_block` the Current latest immutable block.
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
        let params = self
            .current_orphan_state
            .take()
            .and_then(|state| self.should_continue_orphan(state))
            .or_else(|| self.take_next_pending_orphan());

        let Some((orphan_id, additional_blocks, iterations)) = params else {
            return;
        };

        debug!(
            orphan_id = %orphan_id,
            iteration = iterations,
            max_iterations = self.max_number_of_iterations.get(),
            "Starting orphan stream request"
        );

        match self
            .network_adapter
            .request_blocks_from_peers(
                orphan_id,
                local_tip,
                latest_immutable_block,
                additional_blocks,
            )
            .await
        {
            Ok(stream) => {
                self.current_orphan_state = Some(OrphanHandlingState {
                    orphan_id,
                    last_block: None,
                    stream: Some(stream),
                    requests_count: iterations,
                });
            }
            Err(e) => {
                error!(
                    orphan_id = %orphan_id,
                    error = %e,
                    "Failed to request orphan stream"
                );
                self.current_orphan_state = None;
            }
        }
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "We need ownership to destructure and move values out of the state"
    )]
    fn should_continue_orphan(
        &self,
        state: OrphanHandlingState<Tx, BlockCert>,
    ) -> Option<(HeaderId, HashSet<HeaderId>, usize)> {
        let OrphanHandlingState {
            orphan_id: id,
            last_block,
            requests_count: prev_iterations,
            ..
        } = state;

        match last_block {
            Some(last) if last == id => None,
            Some(_) if prev_iterations >= self.max_number_of_iterations.get() => None,
            Some(last) => {
                let mut additional_blocks = HashSet::new();
                additional_blocks.insert(last);

                Some((id, additional_blocks, prev_iterations + 1))
            }
            None => Some((id, HashSet::new(), prev_iterations)),
        }
    }

    fn take_next_pending_orphan(&mut self) -> Option<(HeaderId, HashSet<HeaderId>, usize)> {
        let next = self.pending_orphans.iter().next().copied()?;
        self.pending_orphans.remove(&next);

        Some((next, HashSet::new(), 0))
    }

    async fn poll_stream(&mut self) -> Option<Block<Tx, BlockCert>> {
        let state = self.current_orphan_state.as_mut()?;
        let stream = state.stream.as_mut()?;

        match stream.next().await {
            Some(Ok(block)) => {
                state.last_block = Some(block.header().id());
                self.pending_orphans.remove(&state.orphan_id);

                Some(block)
            }
            Some(Err(e)) => {
                error!(
                    orphan_id = %state.orphan_id,
                    error = %e,
                    "Error while processing orphan stream"
                );
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
        if self.pending_orphans.len() >= self.max_orphan_cache_size.get() {
            debug!(
                orphan_id = ?orphan_id,
                cache_size = self.pending_orphans.len(),
                max_size = self.max_orphan_cache_size.get(),
                "Orphan cache is full, cannot add orphan"
            );

            return;
        }

        if let Some(ref state) = self.current_orphan_state {
            if state.orphan_id == orphan_id {
                return;
            }
        }

        self.pending_orphans.insert(orphan_id);
    }

    pub fn remove_orphan_if_present(&mut self, block_id: &HeaderId) {
        self.pending_orphans.remove(block_id);

        if let Some(ref state) = self.current_orphan_state {
            if &state.orphan_id == block_id {
                self.cancel_active_stream();
            }
        }
    }

    pub fn cancel_active_stream(&mut self) {
        if let Some(state) = self.current_orphan_state.take() {
            debug!(
                orphan_id = %state.orphan_id,
                iterations = state.requests_count,
                "Cancelling active orphan stream"
            );
        } else {
            debug!("No active orphan stream to cancel");
        }
    }

    pub fn should_call(&self) -> bool {
        !self.pending_orphans.is_empty() || self.current_orphan_state.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

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

    type TestBlock = Block<SignedMantleTx, BlobInfo>;
    type OrphanResults = Vec<Result<TestBlock, String>>;

    fn create_chain(size: usize) -> Vec<TestBlock> {
        let proof = make_test_proof();
        let mut chain = Vec::with_capacity(size);

        let mut parent = [0u8; 32].into();
        for _ in 0..size {
            let block = create_block(parent, &proof);
            parent = block.header().id();
            chain.push(block);
        }

        chain
    }

    fn create_block(parent: HeaderId, proof: &Risc0LeaderProof) -> TestBlock {
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

    async fn poll_until_block_or_done<NetAdapter>(
        handler: &mut OrphanHandler<NetAdapter, SignedMantleTx, BlobInfo, usize>,
    ) -> Option<TestBlock>
    where
        NetAdapter: NetworkAdapter<usize, Block = TestBlock> + Send + Sync + Clone,
    {
        timeout(Duration::from_secs(1), async {
            loop {
                match handler
                    .request_blocks([10u8; 32].into(), [11u8; 32].into())
                    .await
                {
                    Some(block) => return Some(block),
                    None => {
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

    fn is_streaming<NetAdapter, Tx, BlockCert, RuntimeServiceId>(
        handler: &OrphanHandler<NetAdapter, Tx, BlockCert, RuntimeServiceId>,
    ) -> bool
    where
        NetAdapter:
            NetworkAdapter<RuntimeServiceId, Block = Block<Tx, BlockCert>> + Send + Sync + Clone,
        Tx: Clone + Eq + Send + Sync + 'static,
        BlockCert: Clone + Eq + Send + Sync + 'static,
        RuntimeServiceId: Send + Sync + 'static,
    {
        handler
            .current_orphan_state
            .as_ref()
            .is_some_and(|state| state.stream.is_some())
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
        orphan_id: HeaderId,
        known_block: Option<HeaderId>,
    }

    #[derive(Clone)]
    struct MockNetworkAdapter {
        requests: Arc<Mutex<Vec<BlockRequest>>>,
        responses: Arc<Mutex<HashMap<ResponseKey, OrphanResults>>>,
    }

    impl MockNetworkAdapter {
        fn new() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn with_initial_stream(
            self,
            orphan_id: HeaderId,
            responses: Vec<Result<TestBlock, String>>,
        ) -> Self {
            let key = ResponseKey {
                orphan_id,
                known_block: None,
            };
            self.responses.lock().unwrap().insert(key, responses);
            self
        }

        fn with_following_stream(
            self,
            orphan_id: HeaderId,
            last_block: HeaderId,
            responses: Vec<Result<TestBlock, String>>,
        ) -> Self {
            let key = ResponseKey {
                orphan_id,
                known_block: Some(last_block),
            };

            self.responses.lock().unwrap().insert(key, responses);

            self
        }
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId: Send + Sync> NetworkAdapter<RuntimeServiceId> for MockNetworkAdapter {
        type Backend = Mock;
        type Settings = ();
        type PeerId = ();
        type Block = TestBlock;

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
            &self,
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

            self.requests.lock().unwrap().push(request);

            let key = ResponseKey {
                orphan_id: target_block,
                known_block: additional_blocks.iter().next().copied(),
            };

            self.responses.lock().unwrap().remove(&key).map_or_else(
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

    const ORPHAN_CACHE_SIZE: NonZeroUsize =
        NonZeroUsize::new(5).expect("ORPHAN_CACHE_SIZE must be non-zero");

    const SECURITY_PARAM: NonZero<u32> =
        NonZero::new(100).expect("SECURITY_PARAM must be non-zero");

    #[tokio::test]
    async fn test_orphan_cache_limit() {
        let network = MockNetworkAdapter::new();
        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        let mut added_orphans = Vec::new();
        for i in 0..handler.max_orphan_cache_size.get() {
            let orphan = [i as u8; 32].into();
            handler.add_orphan(orphan);
            added_orphans.push(orphan);
        }

        assert_eq!(
            handler.pending_orphans.len(),
            handler.max_orphan_cache_size.get()
        );

        for orphan in &added_orphans {
            assert!(handler.pending_orphans.contains(orphan));
        }

        let extra_orphan = [255u8; 32].into();
        handler.add_orphan(extra_orphan);

        assert_eq!(
            handler.pending_orphans.len(),
            handler.max_orphan_cache_size.get()
        );

        assert!(!handler.pending_orphans.contains(&extra_orphan));
    }

    #[tokio::test]
    async fn test_remove_orphan() {
        let network = MockNetworkAdapter::new();
        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        let orphan_id = [1u8; 32].into();
        handler.add_orphan(orphan_id);

        assert!(handler.should_call());

        handler.remove_orphan_if_present(&orphan_id);

        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_has_orphans_with_active_stream() {
        let chain = create_chain(2);
        let orphan_id = chain[1].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, chain.into_iter().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let _ = handler
            .request_blocks([10u8; 32].into(), [11u8; 32].into())
            .await;

        assert_eq!(handler.pending_orphans.len(), 0);
        assert!(handler.should_call());
    }

    #[tokio::test]
    async fn test_next_block_successful_stream() {
        let chain = create_chain(3);
        let orphan_id = chain.last().unwrap().header().id();

        let expected_ids: Vec<_> = chain.iter().map(|b| b.header().id()).collect();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, chain.into_iter().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        for expected_id in &expected_ids {
            let block = poll_until_block_or_done(&mut handler).await;
            assert_eq!(block.as_ref().map(|b| b.header().id()), Some(*expected_id));
        }

        let block = poll_until_block_or_done(&mut handler).await;

        assert!(block.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_next_block_network_error() {
        let orphan_id = [1u8; 32].into();
        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, vec![Err("Network timeout".into())]);

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let block = poll_until_block_or_done(&mut handler).await;

        assert!(block.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_multiple_orphans() {
        let chain = create_chain(10);
        let orphan1_id = chain[2].header().id();
        let orphan2_id = chain[7].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan1_id, chain[0..=2].iter().cloned().map(Ok).collect())
            .with_initial_stream(orphan2_id, chain[5..=7].iter().cloned().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let mut all_blocks = vec![];
        while let Some(block) = poll_until_block_or_done(&mut handler).await {
            all_blocks.push(block);
        }

        let requests = handler.network_adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);

        let first_requested = requests[0].target;
        drop(requests);

        assert_eq!(all_blocks.len(), 6);

        let received_ids: Vec<_> = all_blocks.iter().map(|b| b.header().id()).collect();
        let expected_ids = if first_requested == orphan1_id {
            [&chain[0..=2], &chain[5..=7]].concat()
        } else {
            [&chain[5..=7], &chain[0..=2]].concat()
        };
        let expected_ids: Vec<_> = expected_ids.iter().map(|b| b.header().id()).collect();

        assert_eq!(received_ids, expected_ids);
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_stream_with_errors_midway() {
        let chain = create_chain(3);
        let orphan_id = chain[2].header().id();

        let network = MockNetworkAdapter::new().with_initial_stream(
            orphan_id,
            vec![
                Ok(chain[0].clone()),
                Err("Stream error".into()),
                Ok(chain[1].clone()),
            ],
        );

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let block1 = poll_until_block_or_done(&mut handler).await;

        assert!(block1.is_some());

        let block2 = poll_until_block_or_done(&mut handler).await;

        assert!(block2.is_none());
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_cancel_active_stream() {
        let chain = create_chain(3);
        let orphan_id = chain[2].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, chain.into_iter().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let first = poll_until_block_or_done(&mut handler).await;

        assert!(first.is_some());
        assert!(is_streaming(&handler));

        handler.cancel_active_stream();
        assert!(!is_streaming(&handler));

        let next = poll_until_block_or_done(&mut handler).await;
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn test_cancel_stream_continues_with_next_orphan() {
        let chain = create_chain(10);
        let orphan1_id = chain[2].header().id();
        let orphan2_id = chain[7].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan1_id, chain[0..=2].iter().cloned().map(Ok).collect())
            .with_initial_stream(orphan2_id, chain[5..=7].iter().cloned().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let _ = poll_until_block_or_done(&mut handler).await;

        let first_requested = handler.network_adapter.requests.lock().unwrap()[0].target;

        handler.cancel_active_stream();
        assert!(!is_streaming(&handler));

        let mut blocks_after_cancel = vec![];
        while let Some(block) = poll_until_block_or_done(&mut handler).await {
            blocks_after_cancel.push(block);
        }

        let requests = handler.network_adapter.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);

        let second_requested = requests[1].target;
        drop(requests);

        assert_ne!(first_requested, second_requested);
        assert!(second_requested == orphan1_id || second_requested == orphan2_id);

        assert_eq!(blocks_after_cancel.len(), 3);

        let received_ids: Vec<_> = blocks_after_cancel
            .iter()
            .map(|b| b.header().id())
            .collect();

        let expected_chain = if second_requested == orphan1_id {
            &chain[0..=2]
        } else {
            &chain[5..=7]
        };
        let expected_ids: Vec<_> = expected_chain.iter().map(|b| b.header().id()).collect();

        assert_eq!(received_ids, expected_ids);
        assert!(!handler.should_call());
    }

    #[tokio::test]
    async fn test_following_request() {
        let chain = create_chain(5);
        let orphan_id = chain[4].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, chain[0..3].iter().cloned().map(Ok).collect())
            .with_following_stream(
                orphan_id,
                chain[2].header().id(),
                chain[3..5].iter().cloned().map(Ok).collect(),
            );

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let mut received_blocks = vec![];
        while let Some(block) = poll_until_block_or_done(&mut handler).await {
            received_blocks.push(block);
        }

        assert_eq!(received_blocks.len(), 5);

        for (i, block) in received_blocks.iter().enumerate() {
            assert_eq!(block.header().id(), chain[i].header().id());
        }

        assert!(!handler.should_call());
        assert_eq!(handler.network_adapter.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_max_iterations_limit() {
        let chain = create_chain(10);
        let orphan_id = chain[6].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan_id, chain[0..3].iter().cloned().map(Ok).collect())
            .with_following_stream(
                orphan_id,
                chain[2].header().id(),
                chain[3..=6].iter().cloned().map(Ok).collect(),
            )
            .with_following_stream(
                orphan_id,
                chain[6].header().id(),
                chain[7..].iter().cloned().map(Ok).collect(),
            );

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan_id);

        let mut received_blocks = vec![];
        while let Some(block) = poll_until_block_or_done(&mut handler).await {
            received_blocks.push(block);
        }

        assert_eq!(received_blocks.len(), 7);

        let received_ids: Vec<_> = received_blocks.iter().map(|b| b.header().id()).collect();
        let expected_ids: Vec<_> = chain[0..=6].iter().map(|b| b.header().id()).collect();
        assert_eq!(received_ids, expected_ids);

        assert!(!handler.should_call());
        assert_eq!(handler.network_adapter.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_max_iterations_with_multiple_orphans() {
        let chain = create_chain(15);
        let orphan1_id = chain[4].header().id();
        let orphan2_id = chain[12].header().id();

        let network = MockNetworkAdapter::new()
            .with_initial_stream(orphan1_id, chain[0..2].iter().cloned().map(Ok).collect())
            .with_following_stream(
                orphan1_id,
                chain[1].header().id(),
                chain[2..=4].iter().cloned().map(Ok).collect(),
            )
            .with_following_stream(
                orphan1_id,
                chain[4].header().id(),
                chain[5..7].iter().cloned().map(Ok).collect(),
            )
            .with_initial_stream(orphan2_id, chain[10..=12].iter().cloned().map(Ok).collect());

        let mut handler: OrphanHandler<_, _, _, usize> =
            OrphanHandler::new(network, ORPHAN_CACHE_SIZE, SECURITY_PARAM);

        handler.add_orphan(orphan1_id);
        handler.add_orphan(orphan2_id);

        let mut received_blocks = vec![];
        while let Some(block) = poll_until_block_or_done(&mut handler).await {
            received_blocks.push(block);
        }

        let requests = handler.network_adapter.requests.lock().unwrap();
        let first_orphan = requests[0].target;
        assert_eq!(requests.len(), 3);

        drop(requests);

        assert_eq!(received_blocks.len(), 8);

        let received_ids: Vec<_> = received_blocks.iter().map(|b| b.header().id()).collect();

        let expected_ids = if first_orphan == orphan1_id {
            [&chain[0..=4], &chain[10..=12]].concat()
        } else {
            [&chain[10..=12], &chain[0..=4]].concat()
        };

        let expected_ids: Vec<_> = expected_ids.iter().map(|b| b.header().id()).collect();

        assert_eq!(received_ids, expected_ids);
        assert!(!handler.should_call());
    }
}
