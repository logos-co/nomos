use std::{collections::HashSet, fmt::Debug, hash::Hash};

use cryptarchia_engine::{Boostrapping, Online, Slot};
use futures::StreamExt as _;
use nomos_core::{block::BlockTrait, header::HeaderId};
use overwatch::DynError;
use serde::{Deserialize, Serialize};

use crate::{
    bootstrap::initialization::InitialCryptarchia,
    network::{BoxedStream, NetworkAdapter},
    processor, Cryptarchia,
};

/// Initial Block Download (IBD) for bootstrapping the chain
/// by downloading historical blocks from peers.
pub struct InitialBlockDownload<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::PeerId: Clone + Eq + Hash,
{
    config: IbdConfig<NetAdapter::PeerId>,
}

impl<NetAdapter, RuntimeServiceId> InitialBlockDownload<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Send + Sync,
    NetAdapter::Settings: Send + Sync,
    NetAdapter::Block: BlockTrait,
    NetAdapter::PeerId: Clone + Eq + Hash + Send + Sync,
    RuntimeServiceId: Send + Sync,
{
    /// Creates a [`InitialBlockDownload`] with the given config.
    pub const fn new(config: IbdConfig<NetAdapter::PeerId>) -> Self {
        Self { config }
    }

    /// Runs the IBD with the provided [`InitialCryptarchia`].
    /// This consumes [`InitialCryptarchia`] and returns the new one after
    /// completion.
    pub async fn run<BlockProcessor>(
        self,
        cryptarchia: InitialCryptarchia,
        network_adapter: NetAdapter,
        block_processor: &mut BlockProcessor,
    ) -> Result<IbdCompletedCryptarchia, IbdError>
    where
        BlockProcessor: processor::BlockProcessor<Block = NetAdapter::Block> + Send + Sync,
    {
        match cryptarchia {
            InitialCryptarchia::Bootstrapping(cryptarchia) => self
                .run_with(cryptarchia, network_adapter, block_processor)
                .await
                .map(IbdCompletedCryptarchia::Bootstrapping),
            InitialCryptarchia::Online(cryptarchia) => self
                .run_with(cryptarchia, network_adapter, block_processor)
                .await
                .map(IbdCompletedCryptarchia::Online),
        }
    }

    /// Runs the IBD with the provided [`Cryptarchia`].
    /// This consumes [`Cryptarchia`] and returns the new one after completion.
    async fn run_with<CryptarchiaState, BlockProcessor>(
        self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        network_adapter: NetAdapter,
        block_processor: &mut BlockProcessor,
    ) -> Result<Cryptarchia<CryptarchiaState>, IbdError>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        BlockProcessor: processor::BlockProcessor<Block = NetAdapter::Block> + Send + Sync,
    {
        // TODO: Download from multiple peers.
        //       https://github.com/logos-co/nomos/issues/1455
        match self.config.peers.iter().next() {
            Some(peer) => self
                .download_from_peer(cryptarchia, network_adapter, peer.clone(), block_processor)
                .await
                .map_err(|(_, e)| e),
            None => Ok(cryptarchia),
        }
    }

    /// Downloads blocks from a peer until its tip is reached.
    /// This returns the updated [`Cryptarchia`] after downloading blocks.
    /// If any error occurs during the download,
    /// it returns [`Cryptarchia`] that contains blocks processed before the
    /// error, so that the caller can retry the download.
    async fn download_from_peer<CryptarchiaState, BlockProcessor>(
        &self,
        mut cryptarchia: Cryptarchia<CryptarchiaState>,
        network_adapter: NetAdapter,
        peer: NetAdapter::PeerId,
        block_processor: &mut BlockProcessor,
    ) -> Result<Cryptarchia<CryptarchiaState>, (Cryptarchia<CryptarchiaState>, IbdError)>
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        BlockProcessor: processor::BlockProcessor<Block = NetAdapter::Block> + Send + Sync,
    {
        // Track this to avoid downloading the same block multiple times.
        let mut latest_downloaded_block: Option<HeaderId> = None;

        // Repeat until the peer's tip is reached.
        loop {
            // Each time, set `target` to the most recent peer's tip.
            match network_adapter.request_tip(peer.clone()).await {
                Err(e) => {
                    return Err((cryptarchia, IbdError::ErrorFromBlockProvider(e)));
                }
                Ok(tip) => {
                    if !should_download(tip.id, tip.slot, &cryptarchia) {
                        break;
                    }
                    let target = tip.id;

                    // Download a single stream of blocks from the peer.
                    (cryptarchia, latest_downloaded_block) = self
                        .download_stream(
                            cryptarchia,
                            &network_adapter,
                            peer.clone(),
                            target,
                            latest_downloaded_block,
                            block_processor,
                        )
                        .await?;

                    // Stop if the target block has been downloaded/added.
                    if cryptarchia.consensus.branches().get(&target).is_some() {
                        break;
                    }
                }
            }
        }

        Ok(cryptarchia)
    }

    /// Downloads a single stream of blocks from the peer,
    /// which may or may not contain the target block.
    /// Regardless of whether the target block is reached,
    /// this returns the updated [`Cryptarchia`] and the last block processed.
    /// If an error occurs during the download,
    /// it returns [`Cryptarchia`] that contains blocks processed before the
    /// error.
    async fn download_stream<CryptarchiaState, BlockProcessor>(
        &self,
        cryptarchia: Cryptarchia<CryptarchiaState>,
        network_adapter: &NetAdapter,
        peer: NetAdapter::PeerId,
        target: HeaderId,
        latest_downloaded_block: Option<HeaderId>,
        block_processor: &mut BlockProcessor,
    ) -> Result<
        (Cryptarchia<CryptarchiaState>, Option<HeaderId>),
        (Cryptarchia<CryptarchiaState>, IbdError),
    >
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        BlockProcessor: processor::BlockProcessor<Block = NetAdapter::Block> + Send + Sync,
    {
        // Request a block stream from the peer.
        match network_adapter
            .request_blocks_from_peer(
                peer.clone(),
                target,
                cryptarchia.tip(),
                cryptarchia.lib(),
                latest_downloaded_block.map_or_else(HashSet::new, |id| HashSet::from([id])),
            )
            .await
        {
            Err(e) => Err((cryptarchia, IbdError::ErrorFromBlockProvider(e))),
            Ok(block_stream) => {
                // Process the block stream.
                self.process_block_stream(block_stream, cryptarchia, block_processor)
                    .await
            }
        }
    }

    /// Reads a block stream and processes each block.
    /// This returns the updated [`Cryptarchia`] and the last block processed.
    /// If an error occurs, it returns [`Cryptarchia`] that contains blocks
    /// processed before the error.
    async fn process_block_stream<CryptarchiaState, BlockProcessor>(
        &self,
        mut block_stream: BoxedStream<
            Result<<NetAdapter as NetworkAdapter<RuntimeServiceId>>::Block, DynError>,
        >,
        mut cryptarchia: Cryptarchia<CryptarchiaState>,
        block_processor: &mut BlockProcessor,
    ) -> Result<
        (Cryptarchia<CryptarchiaState>, Option<HeaderId>),
        (Cryptarchia<CryptarchiaState>, IbdError),
    >
    where
        CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        BlockProcessor: processor::BlockProcessor<Block = NetAdapter::Block> + Send + Sync,
    {
        let mut last_block: Option<HeaderId> = None;
        while let Some(res) = block_stream.next().await {
            match res {
                Err(e) => {
                    return Err((cryptarchia, IbdError::ErrorFromBlockProvider(e)));
                }
                Ok(block) => {
                    let id = block.id();
                    // TODO: Refactor `BlockProcessor` to return a result (e.g. invalid block),
                    //       so that we can stop downloading early on error.
                    cryptarchia = block_processor
                        .process_block_and_update_service_state(cryptarchia, block)
                        .await;
                    last_block = Some(id);
                }
            }
        }
        Ok((cryptarchia, last_block))
    }
}

/// Determines whether blocks up to [`target_id`] should be downloaded.
/// Returns `false` if [`target_id`] is already known in the [`Cryptarchia`],
/// or if [`target_slot`] is not greater than the current local LIB slot.
fn should_download<CryptarchiaState>(
    target_id: HeaderId,
    target_slot: Slot,
    cryptarchia: &Cryptarchia<CryptarchiaState>,
) -> bool
where
    CryptarchiaState: cryptarchia_engine::CryptarchiaState,
{
    cryptarchia.consensus.branches().get(&target_id).is_none()
        && cryptarchia.consensus.lib_branch().slot() < target_slot
}

/// Represents the result of the IBD.
pub enum IbdCompletedCryptarchia {
    Bootstrapping(Cryptarchia<Boostrapping>),
    Online(Cryptarchia<Online>),
}

/// IBD configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IbdConfig<PeerId>
where
    PeerId: Clone + Eq + Hash,
{
    pub peers: HashSet<PeerId>,
}

/// IBD error.
#[derive(Debug, thiserror::Error)]
pub enum IbdError {
    #[error("Error from block provider: {0}")]
    ErrorFromBlockProvider(DynError),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        iter::empty,
        marker::PhantomData,
        num::NonZero,
        sync::{Arc, Mutex},
    };

    use cryptarchia_engine::EpochConfig;
    use cryptarchia_sync::GetTipResponse;
    use nomos_ledger::LedgerState;
    use nomos_network::{backends::NetworkBackend, message::ChainSyncEvent, NetworkService};
    use overwatch::{
        overwatch::OverwatchHandle,
        services::{relay::OutboundRelay, ServiceData},
    };
    use tokio_stream::wrappers::BroadcastStream;

    use super::*;
    use crate::processor::BlockProcessor;

    const GENESIS: [u8; 32] = [0; 32];

    #[tokio::test]
    async fn single_stream() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([Ok(Block::new([3; 32], [2; 32], 3))]),
                VecDeque::from([vec![
                    Ok(Block::new([1; 32], GENESIS, 1)),
                    Ok(Block::new([2; 32], [1; 32], 2)),
                    Ok(Block::new([3; 32], [2; 32], 3)),
                ]]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [3; 32].into());
    }

    #[tokio::test]
    async fn multiple_streams() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([6; 32], [5; 32], 6)),
                    Ok(Block::new([6; 32], [5; 32], 6)),
                ]),
                VecDeque::from([
                    vec![
                        Ok(Block::new([1; 32], GENESIS, 1)),
                        Ok(Block::new([2; 32], [1; 32], 2)),
                        Ok(Block::new([3; 32], [2; 32], 3)),
                    ],
                    vec![
                        Ok(Block::new([4; 32], [3; 32], 4)),
                        Ok(Block::new([5; 32], [4; 32], 5)),
                        Ok(Block::new([6; 32], [5; 32], 6)),
                    ],
                ]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [6; 32].into());
    }

    #[tokio::test]
    async fn multiple_streams_with_new_target() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([6; 32], [5; 32], 6)),
                    Ok(Block::new([6; 32], [5; 32], 6)),
                    Ok(Block::new([8; 32], [7; 32], 8)),
                ]),
                VecDeque::from([
                    vec![
                        Ok(Block::new([1; 32], GENESIS, 1)),
                        Ok(Block::new([2; 32], [1; 32], 2)),
                        Ok(Block::new([3; 32], [2; 32], 3)),
                    ],
                    vec![
                        Ok(Block::new([4; 32], [3; 32], 4)),
                        Ok(Block::new([5; 32], [4; 32], 5)),
                        Ok(Block::new([6; 32], [5; 32], 6)),
                    ],
                    vec![
                        Ok(Block::new([7; 32], [6; 32], 7)),
                        Ok(Block::new([8; 32], [7; 32], 8)),
                    ],
                ]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [6; 32].into());
    }

    #[tokio::test]
    async fn new_target_is_already_known() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([6; 32], [5; 32], 6)),
                    // New target is a block already known by the 1st round.
                    Ok(Block::new([3; 32], [2; 32], 3)),
                ]),
                VecDeque::from([vec![
                    Ok(Block::new([1; 32], GENESIS, 1)),
                    Ok(Block::new([2; 32], [1; 32], 2)),
                    Ok(Block::new([3; 32], [2; 32], 3)),
                ]]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [3; 32].into());
    }

    #[tokio::test]
    async fn new_target_is_older_than_lib() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([6; 32], [5; 32], 6)),
                    // New target is older than the expect LIB [3; 32].
                    Ok(Block::new([2; 32], [1; 32], 2)),
                ]),
                VecDeque::from([vec![
                    Ok(Block::new([1; 32], GENESIS, 1)),
                    Ok(Block::new([3; 32], [1; 32], 3)),
                    Ok(Block::new([4; 32], [3; 32], 4)),
                ]]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [4; 32].into());
    }

    #[tokio::test]
    async fn first_stream_is_empty() {
        let mut block_processor = MockBlockProcessor;

        let Ok(cryptarchia) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([3; 32], [2; 32], 3)),
                    Ok(Block::new([3; 32], [3; 32], 3)),
                ]),
                VecDeque::from([
                    // Even if the 1st stream is empty, the download should be repeated.
                    Vec::new(),
                    vec![
                        Ok(Block::new([1; 32], GENESIS, 1)),
                        Ok(Block::new([2; 32], [1; 32], 2)),
                        Ok(Block::new([3; 32], [2; 32], 3)),
                    ],
                ]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should not fail");
        };

        assert_eq!(cryptarchia.consensus.tip(), [3; 32].into());
    }

    #[tokio::test]
    async fn tip_error_from_block_provider() {
        let mut block_processor = MockBlockProcessor;

        let Err((cryptarchia, _)) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([
                    Ok(Block::new([3; 32], [2; 32], 3)),
                    // Tip error in the 2nd round.
                    Err("provider error: tip".into()),
                ]),
                VecDeque::from([
                    vec![
                        Ok(Block::new([1; 32], GENESIS, 1)),
                        Ok(Block::new([2; 32], [1; 32], 2)),
                    ],
                    vec![Ok(Block::new([3; 32], [3; 32], 4))],
                ]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should fail");
        };

        // Cryptarchia holds blocks that were processed before the error.
        assert_eq!(cryptarchia.consensus.tip(), [2; 32].into());
    }

    #[tokio::test]
    async fn block_error_from_block_provider() {
        let mut block_processor = MockBlockProcessor;

        let Err((cryptarchia, _)) = InitialBlockDownload::new(IbdConfig {
            peers: HashSet::new(),
        })
        .download_from_peer(
            new_cryptarchia(),
            MockNetworkAdapter::<()>::new(BlockProvider::new(
                VecDeque::from([Ok(Block::new([3; 32], [2; 32], 3))]),
                VecDeque::from([vec![
                    Ok(Block::new([1; 32], GENESIS, 1)),
                    Ok(Block::new([2; 32], [1; 32], 2)),
                    // Error before returning the target block.
                    Err("provider error: block".into()),
                ]]),
            )),
            (),
            &mut block_processor,
        )
        .await
        else {
            panic!("should fail");
        };

        // Cryptarchia holds blocks that were processed before the error.
        assert_eq!(cryptarchia.consensus.tip(), [2; 32].into());
    }

    fn new_cryptarchia() -> Cryptarchia<Online> {
        Cryptarchia::<_>::from_lib(
            GENESIS.into(),
            LedgerState::from_utxos(empty()),
            nomos_ledger::Config {
                epoch_config: EpochConfig {
                    epoch_stake_distribution_stabilization: NonZero::new(1).unwrap(),
                    epoch_period_nonce_buffer: NonZero::new(1).unwrap(),
                    epoch_period_nonce_stabilization: NonZero::new(1).unwrap(),
                },
                consensus_config: cryptarchia_engine::Config {
                    security_param: NonZero::new(1).unwrap(),
                    active_slot_coeff: 1.0,
                },
            },
        )
    }

    /// A mock block provider that returns the fixed sets of block streams and
    /// tips.
    struct BlockProvider {
        /// Tips to return in sequence.
        tips: VecDeque<Result<Block, DynError>>,
        /// Block streams to return in sequence.
        streams: VecDeque<Vec<Result<Block, DynError>>>,
        /// To memorize the last returned block
        /// for checking the `additional_blocks` in the request.
        last_returned_block: Option<HeaderId>,
    }

    impl BlockProvider {
        fn new(
            tips: VecDeque<Result<Block, DynError>>,
            streams: VecDeque<Vec<Result<Block, DynError>>>,
        ) -> Self {
            Self {
                tips,
                streams,
                last_returned_block: None,
            }
        }
    }

    struct MockNetworkAdapter<RuntimeServiceId> {
        provider: Arc<Mutex<BlockProvider>>,
        _phantom: PhantomData<RuntimeServiceId>,
    }

    impl<RuntimeServiceId> MockNetworkAdapter<RuntimeServiceId> {
        pub fn new(provider: BlockProvider) -> Self {
            Self {
                provider: Arc::new(Mutex::new(provider)),
                _phantom: PhantomData,
            }
        }
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for MockNetworkAdapter<RuntimeServiceId>
    where
        RuntimeServiceId: Send + Sync + 'static,
    {
        type Backend = MockNetworkBackend<RuntimeServiceId>;
        type Settings = ();
        type PeerId = ();
        type Block = Block;

        async fn new(
            _settings: Self::Settings,
            _network_relay: OutboundRelay<
                <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
            >,
        ) -> Self {
            unimplemented!()
        }

        async fn blocks_stream(&self) -> Result<BoxedStream<Self::Block>, DynError> {
            unimplemented!()
        }

        async fn chainsync_events_stream(&self) -> Result<BoxedStream<ChainSyncEvent>, DynError> {
            unimplemented!()
        }

        async fn request_tip(&self, _peer: Self::PeerId) -> Result<GetTipResponse, DynError> {
            // Return the 1st tip left in the provider.
            let tip = self.provider.lock().unwrap().tips.pop_front().unwrap();
            tip.map(|tip| GetTipResponse {
                id: tip.id,
                slot: tip.slot,
            })
        }

        async fn request_blocks_from_peer(
            &self,
            _peer: Self::PeerId,
            _target_block: HeaderId,
            _local_tip: HeaderId,
            _latest_immutable_block: HeaderId,
            additional_blocks: HashSet<HeaderId>,
        ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError> {
            // Return the 1st block stream left in the provider.
            let stream = {
                let mut provider = self.provider.lock().unwrap();

                // Expect that the requester included the last returned block
                // to the `additional_blocks`.
                if let Some(last_block) = provider.last_returned_block {
                    assert!(additional_blocks.contains(&last_block));
                }

                // Prepare the stream to be returned.
                let stream = provider.streams.pop_front().unwrap();

                // Update the last returned block to the last block in the stream.
                if let Some(last_block) = stream.iter().take_while(|result| result.is_ok()).last() {
                    provider.last_returned_block = Some(last_block.as_ref().unwrap().id);
                }

                stream
            };

            Ok(Box::new(tokio_stream::iter(stream.into_iter())))
        }
    }

    struct MockNetworkBackend<RuntimeServiceId> {
        _phantom: PhantomData<RuntimeServiceId>,
    }

    #[async_trait::async_trait]
    impl<RuntimeServiceId> NetworkBackend<RuntimeServiceId> for MockNetworkBackend<RuntimeServiceId>
    where
        RuntimeServiceId: Send + Sync + 'static,
    {
        type Settings = ();
        type Message = ();
        type PubSubEvent = ();
        type ChainSyncEvent = ();

        fn new(
            _config: Self::Settings,
            _overwatch_handle: OverwatchHandle<RuntimeServiceId>,
        ) -> Self {
            unimplemented!()
        }

        async fn process(&self, _msg: Self::Message) {
            unimplemented!()
        }

        async fn subscribe_to_pubsub(&mut self) -> BroadcastStream<Self::PubSubEvent> {
            unimplemented!()
        }

        async fn subscribe_to_chainsync(&mut self) -> BroadcastStream<Self::ChainSyncEvent> {
            unimplemented!()
        }
    }

    /// A mock [`BlockProcessor`] that applies blocks only to the cryptarchia
    /// engine for easy testing.
    struct MockBlockProcessor;

    #[async_trait::async_trait]
    impl BlockProcessor for MockBlockProcessor {
        type Block = Block;

        async fn process_block_and_update_service_state<CryptarchiaState>(
            &mut self,
            cryptarchia: Cryptarchia<CryptarchiaState>,
            block: Self::Block,
        ) -> Cryptarchia<CryptarchiaState>
        where
            CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        {
            self.process_block(cryptarchia, block).await
        }

        async fn process_block<CryptarchiaState>(
            &mut self,
            cryptarchia: Cryptarchia<CryptarchiaState>,
            block: Self::Block,
        ) -> Cryptarchia<CryptarchiaState>
        where
            CryptarchiaState: cryptarchia_engine::CryptarchiaState + Send,
        {
            let mut cryptarchia = cryptarchia;

            match cryptarchia
                .consensus
                .receive_block(block.id, block.parent, block.slot)
            {
                Ok((consensus, _)) => {
                    cryptarchia.consensus = consensus;
                }
                Err(e) => {
                    println!("Failed to process block: {e:?}");
                }
            }

            cryptarchia
        }
    }

    struct Block {
        id: HeaderId,
        parent: HeaderId,
        slot: Slot,
    }

    impl BlockTrait for Block {
        fn id(&self) -> HeaderId {
            self.id
        }
    }

    impl Block {
        fn new(id: [u8; 32], parent: [u8; 32], slot: u64) -> Self {
            Self {
                id: id.into(),
                parent: parent.into(),
                slot: slot.into(),
            }
        }
    }
}
