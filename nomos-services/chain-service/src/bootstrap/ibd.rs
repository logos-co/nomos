use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::{Debug, Formatter},
    future::Future,
    hash::Hash,
    marker::PhantomData,
};

use futures::StreamExt as _;
use nomos_core::header::HeaderId;
use overwatch::DynError;
use tracing::{debug, error, info};

use crate::{
    network::{BoxedStream, NetworkAdapter},
    Cryptarchia, IbdConfig,
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
    NetAdapter::PeerId: Copy + Clone + Eq + Hash + Debug + Send + Sync,
    NetAdapter::Block: Debug,
    ProcessBlockFn:
        Fn(Cryptarchia, HashSet<HeaderId>, NetAdapter::Block) -> ProcessBlockFut + Send + Sync,
    ProcessBlockFut: Future<Output = (Cryptarchia, HashSet<HeaderId>)> + Send,
    RuntimeServiceId: Sync,
{
    /// Runs IBD with the configured peers.
    ///
    /// It downloads blocks from the peers, applies them to the [`Cryptarchia`],
    /// and returns the updated [`Cryptarchia`].
    pub async fn run(
        &self,
        cryptarchia: Cryptarchia,
        storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Result<(Cryptarchia, HashSet<HeaderId>), Error> {
        let downloads = self.initiate_downloads(&cryptarchia).await;
        Ok(self
            .proceed_downloads(downloads, cryptarchia, storage_blocks_to_remove)
            .await)
    }

    /// Initiates downloads from the configured peers.
    async fn initiate_downloads(
        &self,
        cryptarchia: &Cryptarchia,
    ) -> Downloads<NetAdapter::PeerId, NetAdapter::Block> {
        // TODO: Don't initiate multiple downloads for the same target.
        //       https://github.com/logos-co/nomos/issues/1455
        let mut downloads = HashMap::new();
        for peer in &self.config.peers {
            match self.initiate_download(*peer, cryptarchia, None).await {
                Ok(Some(download)) => {
                    info!("Download initiated: {download:?}");
                    downloads.insert(*peer, download);
                }
                Ok(None) => {
                    debug!("No download needed for peer: {peer:?}");
                }
                Err(e) => {
                    error!("Failed to initiate download: peer:{peer:?}: {e}");
                }
            }
        }
        downloads
    }

    /// Initiates a download from a specific peer.
    /// It gets the peer's tip, and requests a block stream to reach the tip.
    /// If the peer's tip already exists in local, no download is initiated
    /// and [`None`] is returned.
    /// If communication fails, an [`Error`] is returned.
    async fn initiate_download(
        &self,
        peer: NetAdapter::PeerId,
        cryptarchia: &Cryptarchia,
        latest_downloaded_block: Option<HeaderId>,
    ) -> Result<Option<Download<NetAdapter::PeerId, NetAdapter::Block>>, Error> {
        // Get the most recent peer's tip and set it as the target.
        let target = self
            .network
            .request_tip(peer)
            .await
            .map_err(Error::BlockProvider)?
            .id;

        if !Self::should_download(&target, cryptarchia) {
            return Ok(None);
        }

        // Request block stream
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

    fn should_download(target: &HeaderId, cryptarchia: &Cryptarchia) -> bool {
        cryptarchia.consensus.branches().get(target).is_none()
    }

    /// Proceeds downloads by reading/processing blocks from the streams.
    /// It initiates/proceeds additional downloads if needed.
    /// It returns the updated [`Cryptarchia`].
    async fn proceed_downloads(
        &self,
        mut downloads: Downloads<NetAdapter::PeerId, NetAdapter::Block>,
        mut cryptarchia: Cryptarchia,
        mut storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> (Cryptarchia, HashSet<HeaderId>) {
        // Read all streams in a round-robin fashion
        // to avoid forming a long chain of blocks received only from a single peer.
        let mut peers: VecDeque<_> = downloads.keys().copied().collect();
        while let Some(peer) = peers.pop_front() {
            let download = downloads.get_mut(&peer).expect("Download must exist");

            // Read one block from the stream.
            match download.next_block().await {
                Ok(Some((_, block))) => {
                    (cryptarchia, storage_blocks_to_remove) =
                        (self.process_block)(cryptarchia, storage_blocks_to_remove, block).await;
                    peers.push_back(peer);
                }
                Ok(None) => {
                    debug!("Download is done. Initiate next download if needed: {peer:?}");
                    if let Some(download) = self
                        .initiate_next_download(
                            downloads.remove(&peer).expect("Download must exist"),
                            &cryptarchia,
                        )
                        .await
                    {
                        downloads.insert(peer, download);
                        peers.push_back(peer);
                    }
                }
                Err(e) => {
                    error!("Download error: Exclude peer({peer:?}) from downloads: {e}");
                    downloads.remove(&peer);
                }
            }
        }

        (cryptarchia, storage_blocks_to_remove)
    }

    async fn initiate_next_download(
        &self,
        prev: Download<NetAdapter::PeerId, NetAdapter::Block>,
        cryptarchia: &Cryptarchia,
    ) -> Option<Download<NetAdapter::PeerId, NetAdapter::Block>> {
        match self
            .initiate_download(prev.peer, cryptarchia, prev.last)
            .await
        {
            Ok(download) => download,
            Err(e) => {
                error!("Failed to initiate next download for {:?}: {e}", prev.peer);
                None
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block provider error: {0}")]
    BlockProvider(DynError),
}

type Downloads<NodeId, Block> = HashMap<NodeId, Download<NodeId, Block>>;

struct Download<NodeId, Block> {
    peer: NodeId,
    target: HeaderId,
    last: Option<HeaderId>,
    stream: BoxedStream<Result<(HeaderId, Block), DynError>>,
}

impl<NodeId, Block> Download<NodeId, Block> {
    fn new(
        peer: NodeId,
        target: HeaderId,
        stream: BoxedStream<Result<(HeaderId, Block), DynError>>,
    ) -> Self {
        Self {
            peer,
            target,
            last: None,
            stream,
        }
    }

    async fn next_block(&mut self) -> Result<Option<(HeaderId, Block)>, DynError> {
        match self.stream.next().await {
            Some(Ok((id, block))) => {
                self.last = Some(id);
                Ok(Some((id, block)))
            }
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }
}

#[expect(clippy::missing_fields_in_debug, reason = "BoxedStream")]
impl<NodeId, Block> Debug for Download<NodeId, Block>
where
    NodeId: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("peer", &self.peer)
            .field("target", &self.target)
            .field("last", &self.last)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{iter::empty, num::NonZero};

    use cryptarchia_engine::{EpochConfig, Slot};
    use cryptarchia_sync::GetTipResponse;
    use nomos_ledger::LedgerState;
    use nomos_network::{backends::NetworkBackend, message::ChainSyncEvent, NetworkService};
    use overwatch::{
        overwatch::OverwatchHandle,
        services::{relay::OutboundRelay, ServiceData},
    };
    use tokio_stream::wrappers::BroadcastStream;

    use super::*;

    #[tokio::test]
    async fn single_download() {
        let peer = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Block::new(2, 1, 2),
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
            Block::new(3, 2, 3),
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
            Block::new(2, 1, 2),
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
            Block::new(5, 4, 5),
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

    #[tokio::test]
    async fn err_from_one_peer() {
        let peer0 = BlockProvider::new(
            vec![
                Block::genesis(),
                Block::new(1, GENESIS_ID, 1),
                Block::new(2, 1, 2),
            ],
            Block::new(2, 1, 2),
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
            Block::new(5, 4, 5),
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
        IbdConfig { peers }
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
        tip: Block,
        stream_limit: usize,
        stream_err: bool,
    }

    impl BlockProvider {
        fn new(chain: Vec<Block>, tip: Block, stream_limit: usize, stream_err: bool) -> Self {
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
            let tip = provider.tip.clone();
            Ok(GetTipResponse {
                id: tip.id,
                slot: tip.slot,
            })
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
            },
            cryptarchia_engine::State::Bootstrapping,
        )
    }
}
