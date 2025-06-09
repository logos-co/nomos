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
    Multiaddr, PeerId, Stream as Libp2pStream, Stream, StreamProtocol,
};
use libp2p_stream::{Behaviour as StreamBehaviour, Control, IncomingStreams};
use nomos_core::header::HeaderId;
use rand::prelude::IteratorRandom as _;
use tokio::sync::mpsc;
use tracing::error;

use crate::{
    blocks_downloader::DownloadBlocksTask,
    blocks_provider::{ProvideBlocksTask, BUFFER_SIZE, MAX_ADDITIONAL_BLOCKS},
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksRequest, RequestMessage},
    SerialisedBlock,
};

/// Cryptarchia networking protocol for synchronizing blocks.
const SYNC_PROTOCOL_ID: &str = "/nomos/cryptarchia/sync/0.1.0";

pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new(SYNC_PROTOCOL_ID);

const MAX_INCOMING_REQUESTS: usize = 4;

type PendingRequestsFuture = BoxFuture<'static, Result<(PeerId, Libp2pStream), ChainSyncError>>;

type LocallyInitiatedDownloadsFuture = BoxStream<'static, Result<BlocksResponse, ChainSyncError>>;

type ExternallyInitiatedDownloadsFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type ExternalPendingDownloadRequestsFuture =
    BoxFuture<'static, Result<(PeerId, Libp2pStream, RequestMessage), ChainSyncError>>;

type ToSwarmEvent = ToSwarm<
    <Behaviour as NetworkBehaviour>::ToSwarm,
    <<Behaviour as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
>;

#[derive(Debug)]
pub enum Event {
    ProvideBlocksRequest {
        /// Return blocks up to `target_block` if specified.
        target_block: HeaderId,
        /// The local canonical chain latest block.
        local_tip: HeaderId,
        /// The latest immutable block.
        latest_immutable_block: HeaderId,
        /// The list of additional blocks that the requester has.
        additional_blocks: Vec<HeaderId>,
        /// Channel to send blocks to the behaviour.
        reply_sender: mpsc::Sender<SerialisedBlock>,
    },
    ProvideTipsRequest {
        /// Channel to send the latest tip to the behaviour.
        reply_sender: mpsc::Sender<HeaderId>,
    },
    DownloadBlocksResponse {
        /// The response containing a block or an error.
        result: BlocksResponse,
    },
}

/// A set of block identifiers the syncing peer already knows.
#[derive(Debug, Clone)]
pub struct DownloadBlocksInfo {
    /// Return blocks up to `target_block` if specified.
    pub target_block: Option<HeaderId>,
    /// The latest block at the tip of the local chain.
    pub local_tip: HeaderId,
    /// The latest immutable block.
    pub latest_immutable_block: HeaderId,
    /// The list of additional blocks that the requester has.
    pub additional_blocks: Vec<HeaderId>,
}

impl Event {
    #[must_use]
    pub const fn provide_blocks_request(
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        reply_sender: mpsc::Sender<SerialisedBlock>,
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

#[derive(Debug)]
pub enum BlocksResponse {
    /// Successful response containing a block.
    Block((PeerId, SerialisedBlock)),
    /// Error happened during block downloading.
    NetworkError(ChainSyncError),
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
    pending_download_requests: FuturesUnordered<PendingRequestsFuture>,
    /// Futures for managing the progress of locally initiated block downloads.
    in_progress_download_requests: SelectAll<LocallyInitiatedDownloadsFuture>,
    /// Futures for reading incoming download requests. After the request is
    /// read, sending blocks is handled by `externally_initiated_downloads`.
    pending_download_responses: FuturesUnordered<ExternalPendingDownloadRequestsFuture>,
    /// Futures for managing the progress of externally initiated block
    /// downloads.
    in_progress_download_responses: FuturesUnordered<ExternallyInitiatedDownloadsFuture>,
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
            in_progress_download_requests: SelectAll::new(),
            in_progress_download_responses: FuturesUnordered::new(),
            pending_download_responses: FuturesUnordered::new(),
            incoming_streams_to_close: FuturesUnordered::new(),
            pending_download_requests: FuturesUnordered::new(),
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
        peer_id: PeerId,
        download_blocks_info: DownloadBlocksInfo,
    ) -> Result<(), ChainSyncError> {
        let peer_id = self.choose_peer().ok_or_else(|| ChainSyncError {
            peer: peer_id,
            kind: ChainSyncErrorKind::StartSyncError("No peers available".to_owned()),
        })?;

        let control = self.control.clone();

        self.pending_download_requests.push(
            async move {
                DownloadBlocksTask::send_request(peer_id, control, download_blocks_info).await
            }
            .boxed(),
        );

        Ok(())
    }

    fn handle_tip_request(&self, peer_id: PeerId, stream: Libp2pStream) -> Poll<ToSwarmEvent> {
        let (reply_sender, reply_rcv) = mpsc::channel(BUFFER_SIZE);

        self.in_progress_download_responses.push(
            async move { ProvideBlocksTask::provide_tip(reply_rcv, peer_id, stream).await }.boxed(),
        );

        Poll::Ready(ToSwarm::GenerateEvent(Event::ProvideTipsRequest {
            reply_sender,
        }))
    }

    fn handle_download_request(
        &self,
        peer_id: PeerId,
        request: DownloadBlocksRequest,
        mut stream: Libp2pStream,
    ) -> Poll<ToSwarmEvent> {
        if request.known_blocks.additional_blocks.len() > MAX_ADDITIONAL_BLOCKS {
            error!("Received excessive number of additional blocks");
            self.incoming_streams_to_close.push(
                async move {
                    let _ = stream.close().await;
                }
                .boxed(),
            );

            return Poll::Pending;
        }

        let (reply_sender, reply_rcv) = mpsc::channel(BUFFER_SIZE);

        self.in_progress_download_responses.push(
            async move { ProvideBlocksTask::provide_blocks(reply_rcv, peer_id, stream).await }
                .boxed(),
        );

        Poll::Ready(ToSwarm::GenerateEvent(Event::provide_blocks_request(
            request.target_block,
            request.known_blocks.local_tip,
            request.known_blocks.latest_immutable_block,
            request.known_blocks.additional_blocks,
            reply_sender,
        )))
    }

    fn handle_incoming_stream(&self, peer_id: PeerId, mut stream: Libp2pStream) {
        if self.pending_download_responses.len() + self.in_progress_download_responses.len()
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
            self.pending_download_responses
                .push(ProvideBlocksTask::process_request(peer_id, stream).boxed());
        }
    }

    fn handle_request_available(
        &mut self,
        result: Result<(PeerId, Stream), ChainSyncError>,
    ) -> Poll<
        ToSwarm<
            <Self as NetworkBehaviour>::ToSwarm,
            <<Self as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
        >,
    > {
        match result {
            Ok((peer_id, stream)) => {
                self.in_progress_download_requests
                    .push(DownloadBlocksTask::download_blocks(peer_id, stream));

                Poll::Pending
            }
            Err(e) => {
                error!("Received error while sending download request: {}", e);

                Poll::Ready(ToSwarm::GenerateEvent(Event::DownloadBlocksResponse {
                    result: BlocksResponse::NetworkError(e),
                }))
            }
        }
    }

    fn handle_request_ready(
        &self,
        cx: &mut Context,
        result: Result<(PeerId, Stream, RequestMessage), ChainSyncError>,
    ) -> Poll<
        ToSwarm<
            <Self as NetworkBehaviour>::ToSwarm,
            <<Self as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
        >,
    > {
        match result {
            Ok((peer_id, stream, request)) => {
                cx.waker().wake_by_ref();
                match request {
                    RequestMessage::DownloadBlocksRequest(request) => {
                        self.handle_download_request(peer_id, request, stream)
                    }
                    RequestMessage::GetTipRequest(_) => self.handle_tip_request(peer_id, stream),
                }
            }
            Err(e) => {
                error!("Error while processing download request: {}", e);
                Poll::Pending
            }
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

        if let Poll::Ready(Some(result)) = self.pending_download_requests.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();
            return self.handle_request_available(result);
        }

        if let Poll::Ready(Some(response)) = self.in_progress_download_requests.poll_next_unpin(cx)
        {
            cx.waker().wake_by_ref();

            match response {
                Ok(result) => {
                    return Poll::Ready(ToSwarm::GenerateEvent(Event::DownloadBlocksResponse {
                        result,
                    }))
                }
                Err(e) => {
                    error!("Error while downloading blocks: {}", e);
                }
            }
        }

        if let Poll::Ready(Some(result)) = self.in_progress_download_responses.poll_next_unpin(cx) {
            if let Err(e) = result {
                error!("Sending response failed: {}", e);
            }

            cx.waker().wake_by_ref();

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.pending_download_responses.poll_next_unpin(cx) {
            return self.handle_request_ready(cx, result);
        }

        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();
            self.handle_incoming_stream(peer_id, stream);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use futures::StreamExt as _;
    use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, Multiaddr, PeerId, Swarm};
    use libp2p_swarm_test::SwarmExt as _;
    use nomos_core::header::HeaderId;
    use rand::{rng, Rng as _};

    use crate::{
        behaviour::{ChainSyncErrorKind, MAX_INCOMING_REQUESTS},
        Behaviour, BlocksResponse, DownloadBlocksInfo, Event,
    };

    #[tokio::test]
    async fn test_block_sync_between_two_swarms() {
        let mut downloader_swarm = start_provider_and_downloader(2).await;

        request_syncs(
            &mut downloader_swarm,
            1,
            &DownloadBlocksInfo {
                target_block: None,
                local_tip: HeaderId::from([0; 32]),
                latest_immutable_block: HeaderId::from([0; 32]),
                additional_blocks: vec![],
            },
        );

        let (blocks, errors) = wait_downloader_events(downloader_swarm, 2).await;

        assert_eq!(blocks.len(), 2);
        assert_eq!(errors.len(), 0);
    }

    #[tokio::test]
    async fn test_download_with_no_peers() {
        let err = Behaviour::new()
            .start_blocks_download(
                PeerId::random(),
                DownloadBlocksInfo {
                    target_block: None,
                    local_tip: HeaderId::from([0; 32]),
                    latest_immutable_block: HeaderId::from([0; 32]),
                    additional_blocks: vec![],
                },
            )
            .unwrap_err();

        matches!(err.kind, ChainSyncErrorKind::StartSyncError(_));
    }

    #[tokio::test]
    async fn test_reject_excess_download_requests() {
        let mut downloader_swarm = start_provider_and_downloader(1).await;

        request_syncs(
            &mut downloader_swarm,
            MAX_INCOMING_REQUESTS + 1,
            &DownloadBlocksInfo {
                target_block: None,
                local_tip: HeaderId::from([0; 32]),
                latest_immutable_block: HeaderId::from([0; 32]),
                additional_blocks: vec![],
            },
        );

        let (_blocks, errors) =
            wait_downloader_events(downloader_swarm, MAX_INCOMING_REQUESTS + 1).await;

        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_reject_protocol_violation_too_many_additional_blocks() {
        let mut downloader_swarm = start_provider_and_downloader(1).await;

        request_syncs(
            &mut downloader_swarm,
            1,
            &DownloadBlocksInfo {
                target_block: None,
                local_tip: HeaderId::from([0; 32]),
                latest_immutable_block: HeaderId::from([0; 32]),
                additional_blocks: vec![],
            },
        );

        let (_blocks, errors) = wait_downloader_events(downloader_swarm, 1).await;

        assert_eq!(errors.len(), 1);
        matches!(&errors[0], BlocksResponse::NetworkError(e) if matches!(e.kind, ChainSyncErrorKind::ProtocolViolation(_)));
    }

    async fn start_provider_and_downloader(blocks_count: usize) -> Swarm<Behaviour> {
        let mut provider_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());
        let provider_addr: Multiaddr = Protocol::Memory(u64::from(rng().random::<u16>())).into();
        provider_swarm.listen_on(provider_addr.clone()).unwrap();

        tokio::spawn(async move {
            while let Some(event) = provider_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::ProvideTipsRequest { reply_sender }) = event {
                    let tip = HeaderId::from([0; 32]);
                    let _ = reply_sender.send(tip).await;
                    continue;
                }
                if let SwarmEvent::Behaviour(Event::ProvideBlocksRequest { reply_sender, .. }) =
                    event
                {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        for _ in 0..blocks_count {
                            let _ = reply_sender.send(Bytes::new()).await;
                        }
                    });
                }
            }
        });

        let mut downloader_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());

        downloader_swarm.dial_and_wait(provider_addr).await;

        downloader_swarm
    }

    fn request_syncs(
        downloader_swarm: &mut Swarm<Behaviour>,
        syncs_count: usize,
        request: &DownloadBlocksInfo,
    ) {
        for _ in 0..syncs_count {
            downloader_swarm
                .behaviour_mut()
                .start_blocks_download(PeerId::random(), request.clone())
                .unwrap();
        }
    }

    async fn wait_downloader_events(
        mut downloader_swarm: Swarm<Behaviour>,
        expected_count: usize,
    ) -> (Vec<BlocksResponse>, Vec<BlocksResponse>) {
        let handle = tokio::spawn(async move {
            let mut blocks = Vec::new();
            let mut errros = Vec::new();
            while let Some(event) = downloader_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::DownloadBlocksResponse { result: response }) =
                    event
                {
                    match &response {
                        BlocksResponse::Block(_) => {
                            blocks.push(response);
                        }
                        BlocksResponse::NetworkError(_) => {
                            errros.push(response);
                        }
                    }

                    if expected_count == blocks.len() + errros.len() {
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
