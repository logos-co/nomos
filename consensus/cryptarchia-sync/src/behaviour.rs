use std::{
    collections::HashSet,
    task::{Context, Poll},
};

use futures::{
    future::BoxFuture,
    stream::{BoxStream, SelectAll},
    AsyncWriteExt as _, FutureExt as _, StreamExt as _,
};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    futures::stream::FuturesUnordered,
    swarm::{
        behaviour::ConnectionEstablished, ConnectionClosed, ConnectionDenied, ConnectionHandler,
        ConnectionId, FromSwarm, NetworkBehaviour, THandlerInEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol,
};
use libp2p_stream::{Behaviour as StreamBehaviour, Control, IncomingStreams};
use nomos_core::header::HeaderId;
use rand::prelude::IteratorRandom as _;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::error;

use crate::{
    blocks_downloader::DownloadBlocksTask,
    blocks_provider::{ProvideBlocksTask, BUFFER_SIZE},
    messages::DownloadBlocksRequest,
    Block,
};

/// Cryptarchia networking protocol for synchronizing blocks.
const SYNC_PROTOCOL_ID: &str = "/nomos/cryptarchia/sync/1.0.0";
pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new(SYNC_PROTOCOL_ID);

const MAX_INCOMING_REQUESTS: usize = 4;

type ToSwarmEvent = ToSwarm<
    <Behaviour as NetworkBehaviour>::ToSwarm,
    <<Behaviour as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
>;

#[derive(Error, Debug)]
pub enum ChainSyncError {
    #[error("Failed to start chain sync: {0}")]
    StartSyncError(String),
    #[error("Peer sent too many blocks (protocol violation): {0}")]
    ProtocolViolation(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Stream error: {0}")]
    OpenStreamError(#[from] libp2p_stream::OpenStreamError),
    #[error("Failed to unpack data from reader: {0}")]
    PackingError(#[from] nomos_core::wire::packing::PackingError),
    #[error("Failed to send data to channel: {0}")]
    TrySendError(#[from] mpsc::error::SendError<BlocksResponse>),
}

#[derive(Debug)]
pub enum Event {
    ProvideBlocksRequest {
        /// Return blocks up to `target_block` if specified.
        target_block: Option<HeaderId>,
        /// The local canonical chain latest block.
        local_tip: HeaderId,
        /// The latest immutable block.
        latest_immutable_block: HeaderId,
        /// The list of additional blocks that the requester has.
        additional_blocks: Vec<HeaderId>,
        /// Channel to send blocks to the behaviour.
        reply_sender: mpsc::Sender<Block>,
    },
    DownloadBlocksResponse {
        /// The response containing a block or an error.
        response: BlocksResponse,
    },
}

impl Event {
    #[must_use]
    pub const fn provide_blocks_request(
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        reply_sender: mpsc::Sender<Block>,
    ) -> Self {
        Self::ProvideBlocksRequest {
            target_block,
            local_tip,
            latest_immutable_block,
            additional_blocks,
            reply_sender,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BlocksResponse {
    /// Successful response containing a block.
    Block(Block),
    /// Error happened during block downloading.
    NetworkError(String),
}

pub struct Behaviour {
    /// The stream behavior to handle actual networking.
    stream_behaviour: StreamBehaviour,
    /// Control to open streams to peers.
    control: Control,
    /// A handle to listen to incoming stream requests.
    incoming_streams: IncomingStreams,
    /// List of connected peers.
    peers: HashSet<PeerId>,
    /// Futures for sending download requests. After the request is
    /// read, sending blocks is handled by `locally_initiated_downloads`.
    locally_pending_download_requests:
        FuturesUnordered<BoxFuture<'static, Result<Stream, ChainSyncError>>>,
    /// Futures for managing the progress of locally initiated block downloads.
    locally_initiated_downloads:
        SelectAll<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>,
    /// Futures for managing the progress of externally initiated block
    /// downloads.
    externally_initiated_downloads:
        FuturesUnordered<BoxFuture<'static, Result<(), ChainSyncError>>>,
    /// Futures for reading incoming download requests. After the request is
    /// read, sending blocks is handled by `externally_initiated_downloads`.
    external_pending_download_requests: FuturesUnordered<
        BoxFuture<'static, Result<(Stream, DownloadBlocksRequest), ChainSyncError>>,
    >,
    /// Futures for closing incoming streams that were rejected due to excess
    /// requests.
    incoming_streams_to_close: FuturesUnordered<BoxFuture<'static, ()>>,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl Behaviour {
    #[must_use]
    pub fn new() -> Self {
        let stream_behaviour = StreamBehaviour::new();
        let mut control = stream_behaviour.new_control();
        let incoming_streams = control
            .accept(SYNC_PROTOCOL)
            .expect("Failed to accept incoming streams for sync protocol");
        Self {
            stream_behaviour,
            control,
            incoming_streams,
            peers: HashSet::new(),
            locally_initiated_downloads: SelectAll::new(),
            externally_initiated_downloads: FuturesUnordered::new(),
            external_pending_download_requests: FuturesUnordered::new(),
            incoming_streams_to_close: FuturesUnordered::new(),
            locally_pending_download_requests: FuturesUnordered::new(),
        }
    }

    fn add_peer(&mut self, peer: PeerId) {
        self.peers.insert(peer);
    }

    fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);
    }

    fn choose_peer(&self) -> Option<PeerId> {
        self.peers.clone().into_iter().choose(&mut rand::rng())
    }

    pub fn start_blocks_download(
        &mut self,
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
    ) -> Result<(), ChainSyncError> {
        let peer_id = self.choose_peer().ok_or_else(|| {
            ChainSyncError::StartSyncError("No peers available for chain sync".into())
        })?;

        let request =
            DownloadBlocksRequest::new(target_block, local_tip, immutable_block, additional_blocks);

        let control = self.control.clone();

        self.locally_pending_download_requests.push(
            async move {
                let stream = DownloadBlocksTask::send_request(peer_id, control, request).await?;
                Ok(stream)
            }
            .boxed(),
        );

        Ok(())
    }

    fn handle_download_request(
        &self,
        result: Result<(Stream, DownloadBlocksRequest), ChainSyncError>,
    ) -> Option<Poll<ToSwarmEvent>> {
        match result {
            Ok((stream, request)) => {
                let (reply_sender, reply_rcv) = mpsc::channel(BUFFER_SIZE);

                self.externally_initiated_downloads.push(
                    async move { ProvideBlocksTask::provide_blocks(reply_rcv, stream).await }
                        .boxed(),
                );

                return Some(Poll::Ready(ToSwarm::GenerateEvent(
                    Event::provide_blocks_request(
                        request.target_block,
                        request.known_blocks.local_tip,
                        request.known_blocks.latest_immutable_block,
                        request.known_blocks.additional_blocks,
                        reply_sender,
                    ),
                )));
            }
            Err(e) => {
                error!("Failed to process download request: {}", e);
            }
        }
        None
    }

    fn handle_incoming_stream(&self, cx: &mut Context, mut stream: Stream) {
        if self.external_pending_download_requests.len() + self.externally_initiated_downloads.len()
            >= MAX_INCOMING_REQUESTS
        {
            self.incoming_streams_to_close.push(
                async move {
                    let _ = stream.close().await;
                }
                .boxed(),
            );
            error!("Rejected excess pending incoming request");
        } else {
            self.external_pending_download_requests
                .push(ProvideBlocksTask::process_download_request(stream).boxed());

            cx.waker().wake_by_ref();
        }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = <StreamBehaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = Event;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.stream_behaviour.handle_pending_inbound_connection(
            connection_id,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.stream_behaviour.handle_established_inbound_connection(
            connection_id,
            peer_id,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer_id: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port: PortUse,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer_id,
                addr,
                role_override,
                port,
            )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished { peer_id, .. }) => {
                self.add_peer(peer_id);
            }
            FromSwarm::ConnectionClosed(ConnectionClosed { peer_id, .. }) => {
                self.remove_peer(&peer_id);
            }
            _ => {}
        }
        self.stream_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        conn_id: ConnectionId,
        event: <Self::ConnectionHandler as ConnectionHandler>::ToBehaviour,
    ) {
        self.stream_behaviour
            .on_connection_handler_event(peer_id, conn_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        while self.incoming_streams_to_close.poll_next_unpin(cx) == Poll::Ready(Some(())) {}

        if let Poll::Ready(Some(result)) =
            self.locally_pending_download_requests.poll_next_unpin(cx)
        {
            match result {
                Ok(stream) => {
                    let stream = DownloadBlocksTask::download_blocks(stream);
                    self.locally_initiated_downloads.push(stream);

                    cx.waker().wake_by_ref();
                }
                Err(e) => {
                    error!("Failed to initiate local download: {}", e);
                }
            }

            return Poll::Pending;
        }

        if let Poll::Ready(Some(response)) = self.locally_initiated_downloads.poll_next_unpin(cx) {
            return match response {
                Ok(result) => {
                    cx.waker().wake_by_ref();

                    Poll::Ready(ToSwarm::GenerateEvent(Event::DownloadBlocksResponse {
                        response: result,
                    }))
                }
                Err(e) => Poll::Ready(ToSwarm::GenerateEvent(Event::DownloadBlocksResponse {
                    response: BlocksResponse::NetworkError(e.to_string()),
                })),
            };
        }

        if let Poll::Ready(Some(result)) = self.externally_initiated_downloads.poll_next_unpin(cx) {
            if let Err(e) = result {
                error!("Sending blocks failed: {}", e);
            }

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) =
            self.external_pending_download_requests.poll_next_unpin(cx)
        {
            if let Some(value) = self.handle_download_request(result) {
                return value;
            }
        }

        if let Poll::Ready(Some((_peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            self.handle_incoming_stream(cx, stream);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use futures::StreamExt as _;
    use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, Multiaddr, Swarm};
    use libp2p_swarm_test::SwarmExt as _;
    use nomos_core::header::HeaderId;
    use rand::{rng, Rng as _};
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use crate::{
        behaviour::MAX_INCOMING_REQUESTS, Behaviour, Block, BlocksResponse, ChainSyncError, Event,
    };

    #[tokio::test]
    async fn test_block_sync_between_two_swarms() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();

        let mut downloader_swarm = start_provider_and_downloader().await;

        request_syncs(&mut downloader_swarm, 1);

        let (blocks, errors) = wait_downloader_events(downloader_swarm, 2).await;

        assert_eq!(blocks.len(), 2);
        assert_eq!(errors.len(), 0);
    }

    #[tokio::test]
    async fn test_download_with_no_peers() {
        let err = Behaviour::new()
            .start_blocks_download(
                None,
                HeaderId::from([0; 32]),
                HeaderId::from([0; 32]),
                vec![],
            )
            .unwrap_err();

        matches!(err, ChainSyncError::StartSyncError(_));
    }

    #[tokio::test]
    async fn test_reject_excess_download_requests() {
        let mut downloader_swarm = start_provider_and_downloader().await;

        request_syncs(&mut downloader_swarm, MAX_INCOMING_REQUESTS + 1);

        let (_blocks, errors) =
            wait_downloader_events(downloader_swarm, MAX_INCOMING_REQUESTS * 2 + 1).await;

        assert_eq!(errors.len(), 1);
    }

    async fn start_provider_and_downloader() -> Swarm<Behaviour> {
        let mut provider_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());
        let provider_addr: Multiaddr = Protocol::Memory(u64::from(rng().random::<u16>())).into();
        provider_swarm.listen_on(provider_addr.clone()).unwrap();

        tokio::spawn(async move {
            while let Some(event) = provider_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::ProvideBlocksRequest { reply_sender, .. }) =
                    event
                {
                    tokio::spawn(async move {
                        for _ in 0..2 {
                            reply_sender.send(Bytes::new()).await.unwrap();
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    });
                }
            }
        });

        let mut downloader_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());

        downloader_swarm.dial_and_wait(provider_addr).await;

        downloader_swarm
    }

    fn request_syncs(downloader_swarm: &mut Swarm<Behaviour>, syncs_count: usize) {
        for _ in 0..syncs_count {
            downloader_swarm
                .behaviour_mut()
                .start_blocks_download(
                    None,
                    HeaderId::from([0; 32]),
                    HeaderId::from([0; 32]),
                    vec![],
                )
                .unwrap();
        }
    }

    async fn wait_downloader_events(
        mut downloader_swarm: Swarm<Behaviour>,
        expected_count: usize,
    ) -> (Vec<Block>, Vec<String>) {
        let handle = tokio::spawn(async move {
            let mut blocks = Vec::new();
            let mut errros = Vec::new();
            while let Some(event) = downloader_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::DownloadBlocksResponse { response }) = event {
                    match response {
                        BlocksResponse::Block(block) => {
                            blocks.push(block);
                        }
                        BlocksResponse::NetworkError(e) => {
                            errros.push(e);
                        }
                    }

                    if expected_count == blocks.len() + errros.len() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        return (blocks, errros);
                    }
                }
            }

            (blocks, errros)
        });

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .unwrap()
            .unwrap()
    }
}
