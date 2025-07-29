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
    IbdConfig,
};

// TODO: Replace ProcessBlock closures with a trait
//       that implements block processing.
//       https://github.com/logos-co/nomos/issues/1505
pub struct InitialBlockDownload<
    NodeId,
    Block,
    NetAdapter,
    Cryptarchia,
    ProcessBlockFn,
    ProcessBlockFut,
    RuntimeServiceId,
> where
    NodeId: Clone + Eq + Hash,
    ProcessBlockFn: Fn(Cryptarchia, Block) -> ProcessBlockFut,
    ProcessBlockFut: Future<Output = Cryptarchia>,
{
    config: IbdConfig<NodeId>,
    network: NetAdapter,
    process_block: ProcessBlockFn,
    _phantom: PhantomData<(NodeId, Block, Cryptarchia, RuntimeServiceId)>,
}

impl<NodeId, Block, NetAdapter, Cryptarchia, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
    InitialBlockDownload<
        NodeId,
        Block,
        NetAdapter,
        Cryptarchia,
        ProcessBlockFn,
        ProcessBlockFut,
        RuntimeServiceId,
    >
where
    NodeId: Clone + Eq + Hash,
    ProcessBlockFn: Fn(Cryptarchia, Block) -> ProcessBlockFut,
    ProcessBlockFut: Future<Output = Cryptarchia>,
{
    pub const fn new(
        config: IbdConfig<NodeId>,
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

impl<NodeId, Block, NetAdapter, Cryptarchia, ProcessBlockFn, ProcessBlockFut, RuntimeServiceId>
    InitialBlockDownload<
        NodeId,
        Block,
        NetAdapter,
        Cryptarchia,
        ProcessBlockFn,
        ProcessBlockFut,
        RuntimeServiceId,
    >
where
    NodeId: Clone + Eq + Hash + Copy + Debug + Send + Sync,
    Block: Send + Sync,
    Cryptarchia: crate::bootstrap::ibd::Cryptarchia + Send + Sync,
    NetAdapter: NetworkAdapter<RuntimeServiceId, PeerId = NodeId, Block = Block> + Send + Sync,
    ProcessBlockFn: Fn(Cryptarchia, Block) -> ProcessBlockFut + Send + Sync,
    ProcessBlockFut: Future<Output = Cryptarchia> + Send,
    RuntimeServiceId: Sync,
{
    /// Runs IBD with the configured peers.
    ///
    /// It downloads blocks from the peers, applies them to the [`Cryptarchia`],
    /// and returns the updated [`Cryptarchia`].
    pub async fn run(&self, cryptarchia: Cryptarchia) -> Result<Cryptarchia, Error> {
        let downloads = self.initiate_downloads(&cryptarchia).await;
        Ok(self.proceed_downloads(downloads, cryptarchia).await)
    }

    /// Initiates downloads from the configured peers.
    async fn initiate_downloads(&self, cryptarchia: &Cryptarchia) -> Downloads<NodeId, Block> {
        let mut downloads = HashMap::new();
        for peer in &self.config.peers {
            match self.initiate_download(*peer, 0, cryptarchia, None).await {
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
    /// If communication fails or if the maximum number of iterations is
    /// reached, an [`Error`] is returned.
    async fn initiate_download(
        &self,
        peer: NodeId,
        iteration: usize,
        cryptarchia: &Cryptarchia,
        latest_downloaded_block: Option<HeaderId>,
    ) -> Result<Option<Download<NodeId, Block>>, Error> {
        if iteration >= self.config.max_download_iterations.get() {
            return Err(Error::MaxDownloadIterationsReached);
        }

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

        Ok(Some(Download::new(peer, iteration, target, stream)))
    }

    fn should_download(target: &HeaderId, cryptarchia: &Cryptarchia) -> bool {
        !cryptarchia.contain(target)
    }

    /// Proceeds downloads by reading/processing blocks from the streams.
    /// It initiates/proceeds additional downloads if needed.
    /// It returns the updated [`Cryptarchia`].
    async fn proceed_downloads(
        &self,
        mut downloads: Downloads<NodeId, Block>,
        mut cryptarchia: Cryptarchia,
    ) -> Cryptarchia {
        // Read all streams in a round-robin fashion
        // to avoid forming a long chain of blocks received only from a single peer.
        let mut peers: VecDeque<_> = downloads.keys().copied().collect();
        while let Some(peer) = peers.pop_front() {
            let download = downloads.get_mut(&peer).expect("Download must exist");

            // Read one block from the stream.
            match download.next_block().await {
                Ok(Some((_, block))) => {
                    cryptarchia = (self.process_block)(cryptarchia, block).await;
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

        cryptarchia
    }

    async fn initiate_next_download(
        &self,
        prev: Download<NodeId, Block>,
        cryptarchia: &Cryptarchia,
    ) -> Option<Download<NodeId, Block>> {
        let next_iteration = prev.iteration.checked_add(1)?;
        match self
            .initiate_download(prev.peer, next_iteration, cryptarchia, prev.last)
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

pub trait Cryptarchia {
    fn tip(&self) -> HeaderId;
    fn lib(&self) -> HeaderId;
    fn contain(&self, id: &HeaderId) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Block provider error: {0}")]
    BlockProvider(DynError),
    #[error("Max download iterations reached")]
    MaxDownloadIterationsReached,
}

type Downloads<NodeId, Block> = HashMap<NodeId, Download<NodeId, Block>>;

struct Download<NodeId, Block> {
    peer: NodeId,
    iteration: usize,
    target: HeaderId,
    last: Option<HeaderId>,
    stream: BoxedStream<Result<(HeaderId, Block), DynError>>,
}

impl<NodeId, Block> Download<NodeId, Block> {
    fn new(
        peer: NodeId,
        iteration: usize,
        target: HeaderId,
        stream: BoxedStream<Result<(HeaderId, Block), DynError>>,
    ) -> Self {
        Self {
            peer,
            iteration,
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
            .field("iteration", &self.iteration)
            .field("target", &self.target)
            .field("last", &self.last)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use cryptarchia_engine::Slot;
    use cryptarchia_sync::GetTipResponse;
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
            vec![Block::new(0, 0), Block::new(1, 1), Block::new(2, 2)],
            Block::new(2, 2),
            2,
            false,
        );
        let local = InitialBlockDownload::new(
            config([NodeId(0)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([(NodeId(0), peer.clone())])),
            process_block,
        )
        .run(Cryptarchia::new())
        .await
        .unwrap();

        // The local chain should be the same as the peer's chain.
        assert_eq!(local.chain, peer.chain);
    }

    #[tokio::test]
    async fn repeat_downloads() {
        let peer = BlockProvider::new(
            vec![
                Block::new(0, 0),
                Block::new(1, 1),
                Block::new(2, 2),
                Block::new(3, 3),
            ],
            Block::new(3, 3),
            2,
            false,
        );
        let local = InitialBlockDownload::new(
            config([NodeId(0)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([(NodeId(0), peer.clone())])),
            process_block,
        )
        .run(Cryptarchia::new())
        .await
        .unwrap();

        // The local chain should be the same as the peer's chain.
        assert_eq!(local.chain, peer.chain);
    }

    #[tokio::test]
    async fn max_download_iterations() {
        let peer = BlockProvider::new(
            vec![
                Block::new(0, 0),
                Block::new(1, 1),
                Block::new(2, 2),
                Block::new(3, 3),
            ],
            Block::new(10, 10),
            2,
            false,
        );
        let local = InitialBlockDownload::new(
            config([NodeId(0)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([(NodeId(0), peer.clone())])),
            process_block,
        )
        .run(Cryptarchia::new())
        .await
        .unwrap();

        // The local chain should be the same as the peer's chain.
        assert_eq!(local.chain, peer.chain);
    }

    #[tokio::test]
    async fn multiple_peers() {
        let peer0 = BlockProvider::new(
            vec![Block::new(0, 0), Block::new(1, 1), Block::new(2, 2)],
            Block::new(2, 2),
            2,
            false,
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::new(0, 0),
                Block::new(3, 3),
                Block::new(4, 4),
                Block::new(5, 5),
            ],
            Block::new(5, 5),
            2,
            false,
        );
        let local = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(Cryptarchia::new())
        .await
        .unwrap();

        // All blocks from both peers should be in the local chain.
        assert!(peer0.chain.iter().all(|b| local.chain.contains(b)));
        assert!(peer1.chain.iter().all(|b| local.chain.contains(b)));
    }

    #[tokio::test]
    async fn err_from_one_peer() {
        let peer0 = BlockProvider::new(
            vec![Block::new(0, 0), Block::new(1, 1), Block::new(2, 2)],
            Block::new(2, 2),
            2,
            true,
        );
        let peer1 = BlockProvider::new(
            vec![
                Block::new(0, 0),
                Block::new(3, 3),
                Block::new(4, 4),
                Block::new(5, 5),
            ],
            Block::new(5, 5),
            2,
            false,
        );
        let local = InitialBlockDownload::new(
            config([NodeId(0), NodeId(1)].into()),
            MockNetworkAdapter::<()>::new(HashMap::from([
                (NodeId(0), peer0.clone()),
                (NodeId(1), peer1.clone()),
            ])),
            process_block,
        )
        .run(Cryptarchia::new())
        .await
        .unwrap();

        // All blocks from peer1 that doesn't return an error
        // should be added to the local chain.
        assert!(peer1.chain.iter().all(|b| local.chain.contains(b)));
    }

    async fn process_block(mut cryptarchia: Cryptarchia, block: Block) -> Cryptarchia {
        cryptarchia.push(block);
        cryptarchia
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct NodeId(usize);

    fn config(peers: HashSet<NodeId>) -> IbdConfig<NodeId> {
        IbdConfig {
            peers,
            download_limit: NonZeroUsize::new(10).unwrap(),
            max_download_iterations: NonZeroUsize::new(10).unwrap(),
        }
    }

    struct Cryptarchia {
        chain: Vec<Block>,
    }

    impl Cryptarchia {
        fn new() -> Self {
            Self {
                chain: vec![Block::new(0, 0)],
            }
        }

        fn push(&mut self, block: Block) {
            self.chain.push(block);
        }
    }

    impl crate::bootstrap::ibd::Cryptarchia for Cryptarchia {
        fn tip(&self) -> HeaderId {
            self.chain.last().unwrap().id
        }

        fn lib(&self) -> HeaderId {
            self.chain.first().unwrap().id
        }

        fn contain(&self, id: &HeaderId) -> bool {
            self.chain.iter().any(|block| &block.id == id)
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Block {
        id: HeaderId,
        slot: Slot,
    }

    impl Block {
        fn new(id: u8, slot: u64) -> Self {
            Self {
                id: [id; 32].into(),
                slot: slot.into(),
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
}
