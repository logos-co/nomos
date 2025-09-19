use std::{collections::HashSet, fmt::Debug, hash::Hash, marker::PhantomData};

use cryptarchia_sync::GetTipResponse;
use futures::StreamExt as _;
use nomos_core::header::HeaderId;
use overwatch::DynError;
use tracing::{debug, error};

use crate::{
    Cryptarchia, IbdConfig,
    bootstrap::download::{Delay, Download, Downloads, DownloadsOutput},
    network::NetworkAdapter,
};

// TODO: Replace ProcessBlock closures with a trait
//       that implements block processing.
//       https://github.com/logos-co/nomos/issues/1505
pub struct InitialBlockDownload<NetAdapter, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::PeerId: Clone + Eq + Hash,
    ProcessBlockFn: Fn(Cryptarchia, HashSet<HeaderId>, NetAdapter::Block) -> ProcessBlockFut,
    ProcessBlockFut: Future<Output = (Cryptarchia, HashSet<HeaderId>)>,
{
    config: IbdConfig<NetAdapter::PeerId>,
    network: NetAdapter,
    process_block: ProcessBlockFn,
    _phantom: PhantomData<RuntimeServiceId>,
}

impl<NetAdapter, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
    InitialBlockDownload<NetAdapter, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId>,
    NetAdapter::PeerId: Clone + Eq + Hash,
    ProcessBlockFn: Fn(Cryptarchia, HashSet<HeaderId>, NetAdapter::Block) -> ProcessBlockFut,
    ProcessBlockFut: Future<Output = (Cryptarchia, HashSet<HeaderId>)>,
{
    pub const fn new(
        config: IbdConfig<NetAdapter::PeerId>,
        network: NetAdapter,
        process_block: ProcessBlockFn,
    ) -> Self {
        Self {
            config,
            network,
            process_block,
            _phantom: PhantomData,
        }
    }
}

impl<NetAdapter, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
    InitialBlockDownload<NetAdapter, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Send + Sync,
    NetAdapter::PeerId: Copy + Clone + Eq + Hash + Debug + Send + Sync + Unpin,
    NetAdapter::Block: Debug + Unpin,
    ProcessBlockFn:
        Fn(Cryptarchia, HashSet<HeaderId>, NetAdapter::Block) -> ProcessBlockFut + Send + Sync,
    ProcessBlockFut: Future<Output = (Cryptarchia, HashSet<HeaderId>)> + Send,
    RuntimeServiceId: Sync,
{
    /// Runs IBD with the configured peers.
    ///
    /// It downloads blocks from the peers, and applies them to the
    /// [`Cryptarchia`].
    ///
    /// An updated [`Cryptarchia`] is returned after downloads from
    /// **all** peers are completed.
    ///
    /// An error is returned if there is no peer available for IBD,
    /// or if all peers return an error.
    pub async fn run(
        &self,
        cryptarchia: Cryptarchia,
        storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Result<(Cryptarchia, HashSet<HeaderId>), Error> {
        if self.config.peers.is_empty() {
            return Ok((cryptarchia, storage_blocks_to_remove));
        }

        let downloads = self.initiate_downloads(&cryptarchia).await?;
        self.proceed_downloads(downloads, cryptarchia, storage_blocks_to_remove)
            .await
    }

    /// Initiates [`Downloads`] from the configured peers.
    async fn initiate_downloads<'a>(
        &self,
        cryptarchia: &Cryptarchia,
    ) -> Result<Downloads<'a, NetAdapter::PeerId, NetAdapter::Block>, Error>
    where
        NetAdapter::PeerId: 'a,
        NetAdapter::Block: 'a,
    {
        let mut downloads = Downloads::new(self.config.delay_before_new_download);
        for peer in &self.config.peers {
            match self
                .initiate_download(*peer, None, cryptarchia, downloads.targets())
                .await
            {
                Ok(Some(download)) => {
                    downloads.add_download(download);
                }
                Ok(None) => {
                    debug!("No download needed for {peer:?}. Delaying the peer");
                    downloads.add_delay(Delay::new(*peer, None));
                }
                Err(e) => {
                    error!("Failed to initiate download for {peer:?}: {e}");
                }
            }
        }

        if downloads.num_peers() == 0 {
            Err(Error::AllPeersFailed)
        } else {
            Ok(downloads)
        }
    }

    /// Initiates a [`Download`] from a specific peer.
    ///
    /// It gets the peer's tip, and requests a block stream to reach the tip.
    ///
    /// If the peer's tip already exists in local, or if there is any duplicate
    /// download for the tip, no download is initiated and [`None`] is returned.
    ///
    /// If communication fails, an [`Error`] is returned.
    async fn initiate_download(
        &self,
        peer: NetAdapter::PeerId,
        latest_downloaded_block: Option<HeaderId>,
        cryptarchia: &Cryptarchia,
        targets_in_progress: &HashSet<HeaderId>,
    ) -> Result<Option<Download<NetAdapter::PeerId, NetAdapter::Block>>, Error> {
        // Get the most recent peer's tip.
        let tip_response = self
            .network
            .request_tip(peer)
            .await
            .map_err(Error::BlockProvider)?;

        // Use the peer's tip as the target for the download.
        let target = match tip_response {
            GetTipResponse::Tip { tip, .. } => tip,
            GetTipResponse::Failure(reason) => {
                return Err(Error::BlockProvider(DynError::from(reason)));
            }
        };

        if !Self::should_download(&target, cryptarchia, targets_in_progress) {
            return Ok(None);
        }

        // Request a block stream.
        let stream = self
            .network
            .request_blocks_from_peer(
                peer,
                target,
                cryptarchia.tip(),
                cryptarchia.lib(),
                latest_downloaded_block.map_or_else(HashSet::new, |id| HashSet::from([id])),
            )
            .await
            .map_err(Error::BlockProvider)?;

        Ok(Some(Download::new(peer, target, stream)))
    }

    fn should_download(
        target: &HeaderId,
        cryptarchia: &Cryptarchia,
        targets_in_progress: &HashSet<HeaderId>,
    ) -> bool {
        cryptarchia.consensus.branches().get(target).is_none()
            && !targets_in_progress.contains(target)
    }

    /// Proceeds [`Downloads`] by reading/processing blocks.
    ///
    /// It returns the updated [`Cryptarchia`] if all downloads have
    /// completed from all peers except the failed ones.
    ///
    /// For peers that complete earlier, delays for the peers are scheduled,
    /// so that new downloads can be initiated after the delays,
    /// as long as there are other peers still in progress.
    ///
    /// An error is return if all peers fail.
    async fn proceed_downloads<'a>(
        &self,
        mut downloads: Downloads<'a, NetAdapter::PeerId, NetAdapter::Block>,
        mut cryptarchia: Cryptarchia,
        mut storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Result<(Cryptarchia, HashSet<HeaderId>), Error>
    where
        NetAdapter::PeerId: 'a,
        NetAdapter::Block: 'a,
    {
        // Track failed peers, so that we can return an error if all peers fail.
        let num_peers = downloads.num_peers();
        let mut failed_peers = HashSet::new();

        // Repeat until there is no download remaining.
        while let Some(output) = downloads.next().await {
            match output {
                DownloadsOutput::DelayCompleted(delay) => {
                    downloads = self
                        .try_initiate_download(
                            *delay.peer(),
                            delay.latest_downloaded_block(),
                            &cryptarchia,
                            downloads,
                        )
                        .await;
                }
                DownloadsOutput::BlockReceived { block, download } => {
                    (cryptarchia, storage_blocks_to_remove) =
                        (self.process_block)(cryptarchia, storage_blocks_to_remove, block).await;
                    // TODO: Stop download if process_block fails.
                    //       (Requires refactoring the chain service)
                    // TODO: Close the download (the underlying stream) when we need to stop it.
                    //       https://github.com/logos-co/nomos/issues/1517
                    downloads.add_download(download);
                }
                DownloadsOutput::DownloadCompleted(download) => {
                    debug!(
                        "A download completed for {:?}. Try a new download",
                        download.peer()
                    );
                    downloads = self
                        .try_initiate_download(
                            *download.peer(),
                            download.last(),
                            &cryptarchia,
                            downloads,
                        )
                        .await;
                }
                DownloadsOutput::Error { error, download } => {
                    error!("Download failed from {:?}: {}", download.peer(), error);
                    failed_peers.insert(*download.peer());
                    if failed_peers.len() == num_peers {
                        return Err(Error::AllPeersFailed);
                    }
                }
            }
        }

        Ok((cryptarchia, storage_blocks_to_remove))
    }

    /// Tries to initiate a download for a peer.
    ///
    /// If there is no download needed at the moment, a delay is scheduled,
    /// so that a new download can be attempted later.
    ///
    /// The peer is ignored if the communication fails.
    async fn try_initiate_download<'a>(
        &self,
        peer: NetAdapter::PeerId,
        latest_downloaded_block: Option<HeaderId>,
        cryptarchia: &Cryptarchia,
        mut downloads: Downloads<'a, NetAdapter::PeerId, NetAdapter::Block>,
    ) -> Downloads<'a, NetAdapter::PeerId, NetAdapter::Block>
    where
        NetAdapter::PeerId: 'a,
        NetAdapter::Block: 'a,
    {
        match self
            .initiate_download(
                peer,
                latest_downloaded_block,
                cryptarchia,
                downloads.targets(),
            )
            .await
        {
            Ok(Some(download)) => {
                downloads.add_download(download);
            }
            Ok(None) => {
                downloads.add_delay(Delay::new(peer, latest_downloaded_block));
            }
            Err(e) => {
                error!("Failed to initiate next download for {peer:?}: {e}");
            }
        }
        downloads
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block provider error: {0}")]
    BlockProvider(DynError),
    #[error("All peers failed")]
    AllPeersFailed,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, iter::empty, num::NonZero};

    use cryptarchia_engine::{EpochConfig, Slot};
    use nomos_core::sdp::{MinStake, ServiceParameters};
    use nomos_ledger::LedgerState;
    use nomos_network::{NetworkService, backends::NetworkBackend, message::ChainSyncEvent};
    use overwatch::{
        overwatch::OverwatchHandle,
        services::{ServiceData, relay::OutboundRelay},
    };
    use tokio_stream::wrappers::BroadcastStream;

    use super::*;
    use crate::network::BoxedStream;

    #[tokio::test]
    async fn no_peers_configured() {
        let (cryptarchia, _) = InitialBlockDownload::new(
            config(HashSet::new()),
            MockNetworkAdapter::<()>::new(HashMap::new()),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // The Cryptarchia remains unchanged.
        assert_eq!(cryptarchia.lib(), [GENESIS_ID; 32].into());
        assert_eq!(cryptarchia.tip(), [GENESIS_ID; 32].into());
    }

    #[tokio::test]
    async fn single_download() {
        let peer = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Ok(Block::new(2, 1, 2)),
            2,
            false,
        );
        let (cryptarchia, _) = InitialBlockDownload::new(
            config([NodeId(0)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([(NodeId(0), peer.clone())])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // All blocks from the peer should be in the local chain.
        assert!(peer.chain.iter().all(|b| contain(b, &cryptarchia)));
    }

    #[tokio::test]
    async fn repeat_downloads() {
        let peer = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
                Block::new(3, 2, 3),
            ],
            Ok(Block::new(3, 2, 3)),
            2,
            false,
        );
        let (cryptarchia, _) = InitialBlockDownload::new(
            config([NodeId(0)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([(NodeId(0), peer.clone())])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // All blocks from the peer should be in the local chain.
        assert!(peer.chain.iter().all(|b| contain(b, &cryptarchia)));
    }

    #[tokio::test]
    async fn multiple_peers() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Ok(Block::new(2, 1, 2)),
            2,
            false,
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(3, GENESIS_ID, 3),
                Block::new(4, 3, 4),
                Block::new(5, 4, 5),
            ],
            Ok(Block::new(5, 4, 5)),
            2,
            false,
        );
        let (cryptarchia, _) = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // All blocks from both peers should be in the local chain.
        assert!(peer0.chain.iter().all(|b| contain(b, &cryptarchia)));
        assert!(peer1.chain.iter().all(|b| contain(b, &cryptarchia)));
    }

    /// If one peer returns an error while streaming blocks,
    /// the peer should be ignored, and IBD should continue
    /// with the remaining peers.
    #[tokio::test]
    async fn err_from_one_peer_while_downloading() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Ok(Block::new(2, 1, 2)),
            2,
            true, // Return error while streaming blocks
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(3, GENESIS_ID, 3),
                Block::new(4, 3, 4),
                Block::new(5, 4, 5),
            ],
            Ok(Block::new(5, 4, 5)),
            2,
            false,
        );
        let (cryptarchia, _) = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // All blocks from peer1 that doesn't return an error
        // should be added to the local chain.
        assert!(peer1.chain.iter().all(|b| contain(b, &cryptarchia)));
    }

    /// If all peers return an error while streaming blocks,
    /// [`Error::AllPeersFailed`] should be returned.
    #[tokio::test]
    async fn err_from_all_peers_while_downloading() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Ok(Block::new(2, 1, 2)),
            2,
            true, // Return error while streaming blocks
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(3, GENESIS_ID, 3),
                Block::new(4, 3, 4),
                Block::new(5, 4, 5),
            ],
            Ok(Block::new(5, 4, 5)),
            2,
            true, // Return error while streaming blocks
        );
        let result = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await;

        assert!(matches!(result, Err(Error::AllPeersFailed)));
    }

    /// If one peer returns an error while initiating download,
    /// the peer should be ignored, and IBD should continue
    /// with the remaining peers.
    #[tokio::test]
    async fn err_from_one_peer_while_initiating() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Err(()), // Return error while initiating download
            2,
            true,
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(3, GENESIS_ID, 3),
                Block::new(4, 3, 4),
                Block::new(5, 4, 5),
            ],
            Ok(Block::new(5, 4, 5)),
            2,
            false,
        );
        let (cryptarchia, _) = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await
        .unwrap();

        // All blocks from peer1 that doesn't return an error
        // should be added to the local chain.
        assert!(peer1.chain.iter().all(|b| contain(b, &cryptarchia)));
    }

    /// If all peers return an error while initiating download,
    /// [`Error::AllPeersFailed`] should be returned.
    #[tokio::test]
    async fn err_from_all_peers_while_initiating() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Err(()), // Return error while initiating download
            2,
            true,
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(3, GENESIS_ID, 3),
                Block::new(4, 3, 4),
                Block::new(5, 4, 5),
            ],
            Err(()), // Return error while initiating download
            2,
            true,
        );
        let result = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(new_cryptarchia(), HashSet::new())
        .await;

        assert!(matches!(result, Err(Error::AllPeersFailed)));
    }

    async fn process_block(
        mut cryptarchia: Cryptarchia,
        storage_blocks_to_remove: HashSet<HeaderId>,
        block: Block,
    ) -> (Cryptarchia, HashSet<HeaderId>) {
        // Add the block only to the consensus, not to the ledger state
        // because the mocked block doesn't have a proof.
        // It's enough because the tests doesn't check the ledger state.
        let (consensus, _) = cryptarchia
            .consensus
            .receive_block(block.id, block.parent, block.slot)
            .expect("Block must be valid");
        cryptarchia.consensus = consensus;
        (cryptarchia, storage_blocks_to_remove)
    }

    fn contain(block: &Block, cryptarchia: &Cryptarchia) -> bool {
        cryptarchia.consensus.branches().get(&block.id).is_some()
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct NodeId(usize);

    fn config(peers: HashSet<NodeId>) -> IbdConfig<NodeId> {
        IbdConfig {
            peers,
            delay_before_new_download: std::time::Duration::from_millis(1),
        }
    }

    const GENESIS_ID: u8 = 0;

    #[derive(Clone, Debug, PartialEq)]
    struct Block {
        id: HeaderId,
        parent: HeaderId,
        slot: Slot,
    }

    impl Block {
        fn new(id: u8, parent: u8, slot: u64) -> Self {
            Self {
                id: [id; 32].into(),
                parent: [parent; 32].into(),
                slot: slot.into(),
            }
        }

        fn genesis() -> Self {
            Self {
                id: [GENESIS_ID; 32].into(),
                parent: [GENESIS_ID; 32].into(),
                slot: Slot::genesis(),
            }
        }
    }

    /// A mock block provider that returns the fixed sets of block streams.
    #[derive(Clone)]
    struct BlockProvider {
        chain: Vec<Block>,
        tip: Result<Block, ()>,
        stream_limit: usize,
        stream_err: bool,
    }

    impl BlockProvider {
        fn new(
            chain: Vec<Block>,
            tip: Result<Block, ()>,
            stream_limit: usize,
            stream_err: bool,
        ) -> Self {
            Self {
                chain,
                tip,
                stream_limit,
                stream_err,
            }
        }

        fn stream(&self, known_blocks: &HashSet<HeaderId>) -> Vec<Result<Block, DynError>> {
            if self.stream_err {
                return vec![Err(DynError::from("Stream error"))];
            }

            let start_pos = self
                .chain
                .iter()
                .rposition(|block| known_blocks.contains(&block.id))
                .map_or(0, |pos| pos + 1);
            if start_pos >= self.chain.len() {
                vec![]
            } else {
                self.chain[start_pos..]
                    .iter()
                    .take(self.stream_limit)
                    .cloned()
                    .map(Ok)
                    .collect()
            }
        }
    }

    /// A mock network adapter that returns a static set of blocks.
    struct MockNetworkAdapter<RuntimeServiceId> {
        providers: HashMap<NodeId, BlockProvider>,
        _phantom: PhantomData<RuntimeServiceId>,
    }

    impl<RuntimeServiceId> MockNetworkAdapter<RuntimeServiceId> {
        pub fn new(providers: HashMap<NodeId, BlockProvider>) -> Self {
            Self {
                providers,
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
        type PeerId = NodeId;
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

        async fn request_tip(&self, peer: Self::PeerId) -> Result<GetTipResponse, DynError> {
            let provider = self.providers.get(&peer).unwrap();
            match provider.tip.clone() {
                Ok(tip) => Ok(GetTipResponse::Tip {
                    tip: tip.id,
                    slot: tip.slot,
                }),
                Err(()) => Err(DynError::from("Cannot provide tip")),
            }
        }

        async fn request_blocks_from_peer(
            &self,
            peer: Self::PeerId,
            _target_block: HeaderId,
            local_tip: HeaderId,
            latest_immutable_block: HeaderId,
            additional_blocks: HashSet<HeaderId>,
        ) -> Result<BoxedStream<Result<(HeaderId, Self::Block), DynError>>, DynError> {
            let provider = self.providers.get(&peer).unwrap();

            let mut known_blocks = additional_blocks;
            known_blocks.insert(local_tip);
            known_blocks.insert(latest_immutable_block);

            let stream = provider.stream(&known_blocks);
            Ok(Box::new(tokio_stream::iter(stream.into_iter().map(
                |result| match result {
                    Ok(block) => Ok((block.id, block)),
                    Err(e) => Err(e),
                },
            ))))
        }

        async fn request_blocks_from_peers(
            &self,
            _target_block: HeaderId,
            _local_tip: HeaderId,
            _latest_immutable_block: HeaderId,
            _additional_blocks: HashSet<HeaderId>,
        ) -> Result<BoxedStream<Result<(HeaderId, Self::Block), DynError>>, DynError> {
            unimplemented!()
        }
    }

    /// A mock network backend that does nothing.
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

    fn new_cryptarchia() -> Cryptarchia {
        Cryptarchia::from_lib(
            [GENESIS_ID; 32].into(),
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
                service_params: ServiceParameters {
                    lock_period: 10,
                    inactivity_period: 20,
                    retention_period: 100,
                    timestamp: 0,
                },
                min_stake: MinStake {
                    threshold: 1,
                    timestamp: 0,
                },
            },
            cryptarchia_engine::State::Bootstrapping,
        )
    }
}
