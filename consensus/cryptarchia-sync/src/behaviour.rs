use std::{
    collections::HashSet,
    task::{Context, Poll},
};

use futures::{
    future::BoxFuture, stream::BoxStream, AsyncWriteExt as _, FutureExt as _, StreamExt as _,
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
use tokio::sync::{mpsc, mpsc::Sender, oneshot};
use tracing::error;

use crate::{
    downloader::Downloader,
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksRequest, RequestMessage, SerialisedHeaderId},
    provider::{Provider, MAX_ADDITIONAL_BLOCKS},
    SerialisedBlock,
};

/// Cryptarchia networking protocol for synchronizing blocks.
const SYNC_PROTOCOL_ID: &str = "/nomos/cryptarchia/sync/0.1.0";

pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new(SYNC_PROTOCOL_ID);

const MAX_INCOMING_REQUESTS: usize = 4;

type SendingRequestsFuture = BoxFuture<'static, Result<RequestStream, ChainSyncError>>;

type ReceivingResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type SendingResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type ReceivingRequestsFuture = BoxFuture<'static, Result<ResponseStream, ChainSyncError>>;

type ToSwarmEvent = ToSwarm<
    <Behaviour as NetworkBehaviour>::ToSwarm,
    <<Behaviour as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
>;

/// Uniform interface for channels so we can have a common type for
/// `SendingRequestsFuture`
pub enum ReplyChannel {
    Blocks(Sender<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>),
    Tip(oneshot::Sender<TipResponse>),
}

pub struct RequestStream {
    pub peer_id: PeerId,
    pub stream: Libp2pStream,
    pub request: RequestMessage,
    pub reply_channel: ReplyChannel,
}

impl RequestStream {
    pub const fn new(
        peer_id: PeerId,
        stream: Stream,
        request: RequestMessage,
        reply_channel: ReplyChannel,
    ) -> Self {
        Self {
            peer_id,
            stream,
            request,
            reply_channel,
        }
    }
}

pub struct ResponseStream {
    pub peer_id: PeerId,
    pub stream: Libp2pStream,
    pub request: RequestMessage,
}

impl ResponseStream {
    pub const fn new(peer_id: PeerId, stream: Stream, request: RequestMessage) -> Self {
        Self {
            peer_id,
            stream,
            request,
        }
    }
}

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
        reply_sender: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
    },
    ProvideTipsRequest {
        /// Channel to send the latest tip to the behaviour.
        reply_sender: oneshot::Sender<SerialisedHeaderId>,
    },
    DownloadBlocksResponse {
        /// The response containing a block or an error.
        result: BlocksResponse,
    },
    GetTipResponse {
        /// Local tip.
        result: TipResponse,
    },
}

#[derive(Debug)]
pub enum BlocksResponse {
    /// Successful response containing a block.
    Block((PeerId, SerialisedBlock)),
    /// Error happened during block downloading.
    NetworkError(ChainSyncError),
}

#[derive(Debug)]
pub enum TipResponse {
    /// Successful response containing the tip of the peer.
    Tip((PeerId, SerialisedHeaderId)),
    /// Error happened while getting the tip.
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
    /// read, reading blocks is handled by `in_progress_download_requests`.
    sending_requests: FuturesUnordered<SendingRequestsFuture>,
    /// Futures for managing the progress of locally initiated block downloads.
    receiving_responses: FuturesUnordered<ReceivingResponsesFuture>,
    /// Futures for reading incoming download requests. After the request is
    /// read, sending blocks is handled by `in_progress_download_responses`.
    receiving_requests: FuturesUnordered<ReceivingRequestsFuture>,
    /// Futures for managing the progress of externally initiated block
    /// downloads.
    sending_responses: FuturesUnordered<SendingResponsesFuture>,
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
            receiving_responses: FuturesUnordered::new(),
            sending_responses: FuturesUnordered::new(),
            receiving_requests: FuturesUnordered::new(),
            incoming_streams_to_close: FuturesUnordered::new(),
            sending_requests: FuturesUnordered::new(),
        }
    }

    fn add_peer(&mut self, peer: PeerId) {
        self.peers.insert(peer);
    }

    fn remove_peer(&mut self, peer: &PeerId) {
        self.peers.remove(peer);
    }

    pub fn request_tip(
        &mut self,
        peer_id: PeerId,
        reply_sender: oneshot::Sender<TipResponse>,
    ) -> Result<(), ChainSyncError> {
        if !self.peers.contains(&peer_id) {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::RequestTipsError("Peer is not connected".to_owned()),
            });
        }

        let mut control = self.control.clone();

        self.sending_requests.push(
            async move { Downloader::send_tip_request(peer_id, &mut control, reply_sender).await }
                .boxed(),
        );

        Ok(())
    }

    pub fn start_blocks_download(
        &mut self,
        peer_id: PeerId,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        reply_sender: Sender<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>,
    ) -> Result<(), ChainSyncError> {
        if !self.peers.contains(&peer_id) {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::StartSyncError("Peer is not connected".to_owned()),
            });
        }

        let control = self.control.clone();

        let request = DownloadBlocksRequest::new(
            target_block,
            local_tip,
            latest_immutable_block,
            additional_blocks,
        );

        self.sending_requests.push(
            Downloader::send_download_request(peer_id, control, request, reply_sender).boxed(),
        );

        Ok(())
    }

    fn handle_tip_request(&self, peer_id: PeerId, stream: Libp2pStream) -> Poll<ToSwarmEvent> {
        let (reply_sender, reply_rcv) = oneshot::channel();

        self.sending_responses
            .push(async move { Provider::provide_tip(reply_rcv, peer_id, stream).await }.boxed());

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

        let (reply_sender, reply_rcv) = mpsc::channel(1);

        self.sending_responses.push(
            async move { Provider::provide_blocks(reply_rcv, peer_id, stream).await }.boxed(),
        );

        Poll::Ready(ToSwarm::GenerateEvent(Event::ProvideBlocksRequest {
            target_block: request.target_block,
            local_tip: request.known_blocks.local_tip,
            latest_immutable_block: request.known_blocks.latest_immutable_block,
            additional_blocks: request.known_blocks.additional_blocks,
            reply_sender,
        }))
    }

    fn handle_incoming_stream(&self, cx: &Context<'_>, peer_id: PeerId, mut stream: Libp2pStream) {
        if self.receiving_requests.len() + self.sending_responses.len() >= MAX_INCOMING_REQUESTS {
            self.incoming_streams_to_close.push(
                async move {
                    let _ = stream.close().await;
                }
                .boxed(),
            );
            error!("Rejected excess pending incoming request");
        } else {
            self.receiving_requests
                .push(Provider::process_request(peer_id, stream).boxed());
            cx.waker().wake_by_ref();
        }
    }

    fn handle_tip_request_available(&self, cx: &Context<'_>, request_stream: RequestStream) {
        self.receiving_responses
            .push(Downloader::receive_tip(request_stream).boxed());
        cx.waker().wake_by_ref();
    }

    fn handle_blocks_request_available(&self, cx: &Context<'_>, request_stream: RequestStream) {
        self.receiving_responses
            .push(Downloader::receive_blocks(request_stream).boxed());

        cx.waker().wake_by_ref();
    }

    fn handle_request_ready(
        &self,
        cx: &Context<'_>,
        response_stream: ResponseStream,
    ) -> Poll<
        ToSwarm<
            <Self as NetworkBehaviour>::ToSwarm,
            <<Self as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
        >,
    > {
        let event = match response_stream.request {
            RequestMessage::DownloadBlocksRequest(request) => self.handle_download_request(
                response_stream.peer_id,
                request,
                response_stream.stream,
            ),
            RequestMessage::GetTip => {
                self.handle_tip_request(response_stream.peer_id, response_stream.stream)
            }
        };

        cx.waker().wake_by_ref();

        event
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

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[Multiaddr],
        effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        self.stream_behaviour.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
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

    #[expect(
        clippy::cognitive_complexity,
        reason = "It contains only basic polling logic"
    )]
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        while self.incoming_streams_to_close.poll_next_unpin(cx) == Poll::Ready(Some(())) {}

        if let Poll::Ready(Some(result)) = self.sending_requests.poll_next_unpin(cx) {
            match result {
                Ok(request_stream) => match request_stream.request {
                    RequestMessage::DownloadBlocksRequest(_) => {
                        self.handle_blocks_request_available(cx, request_stream);
                    }
                    RequestMessage::GetTip => {
                        self.handle_tip_request_available(cx, request_stream);
                    }
                },
                Err(e) => {
                    error!("Error while processing download request: {}", e);
                }
            }

            return Poll::Pending;
        }

        if let Poll::Ready(Some(_)) = self.receiving_responses.poll_next_unpin(cx) {
            cx.waker().wake_by_ref();

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.sending_responses.poll_next_unpin(cx) {
            if let Err(e) = result {
                error!("Sending response failed: {}", e);
            }

            cx.waker().wake_by_ref();

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.receiving_requests.poll_next_unpin(cx) {
            match result {
                Ok(request_stream) => return self.handle_request_ready(cx, request_stream),
                Err(e) => {
                    error!("Error while processing incoming request: {}", e);
                }
            }
        }

        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            self.handle_incoming_stream(cx, peer_id, stream);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use futures::{stream::BoxStream, StreamExt as _};
    use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, Multiaddr, PeerId, Swarm};
    use libp2p_swarm_test::SwarmExt as _;
    use nomos_core::header::HeaderId;
    use rand::{rng, Rng as _};
    use tokio::sync::{mpsc, oneshot};

    use crate::{
        behaviour::{ChainSyncErrorKind, TipResponse, MAX_INCOMING_REQUESTS},
        provider::MAX_ADDITIONAL_BLOCKS,
        Behaviour, BlocksResponse, ChainSyncError, Event,
    };

    type BlocksResponseChannel = (
        mpsc::Sender<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>,
        mpsc::Receiver<BoxStream<'static, Result<BlocksResponse, ChainSyncError>>>,
    );

    #[tokio::test]
    async fn test_block_sync_between_two_swarms() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(200).await;

        let mut streams = request_download(
            &mut downloader_swarm,
            1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &[],
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (blocks, errors) = wait_block_messages(200, &mut streams).await;

        assert_eq!(blocks.len(), 200);
        assert_eq!(errors.len(), 0);
    }

    #[tokio::test]
    async fn test_download_with_no_peers() {
        let (response_tx, _response_rx) = mpsc::channel(1);
        let err = Behaviour::new()
            .start_blocks_download(
                PeerId::random(),
                HeaderId::from([0; 32]),
                HeaderId::from([0; 32]),
                HeaderId::from([0; 32]),
                vec![],
                response_tx,
            )
            .unwrap_err();

        matches!(err.kind, ChainSyncErrorKind::StartSyncError(_));
    }

    #[tokio::test]
    async fn test_reject_excess_download_requests() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(1).await;

        let mut streams = request_download(
            &mut downloader_swarm,
            MAX_INCOMING_REQUESTS + 1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &[],
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (_blocks, errors) = wait_block_messages(1, &mut streams).await;

        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_reject_protocol_violation_too_many_additional_blocks() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(1).await;

        let mut streams = request_download(
            &mut downloader_swarm,
            MAX_INCOMING_REQUESTS + 1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &[HeaderId::from([1; 32]); MAX_ADDITIONAL_BLOCKS + 1],
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (_blocks, errors) = wait_block_messages(1, &mut streams).await;

        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_get_tip() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(0).await;

        let receiver = request_tip(&mut downloader_swarm, provider_peer_id);

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let tip = receiver.await.unwrap();
        assert!(matches!(tip, TipResponse::Tip((_, tip_id)) if tip_id == Bytes::new()));
    }

    async fn start_provider_and_downloader(blocks_count: usize) -> (Swarm<Behaviour>, PeerId) {
        let mut provider_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());
        let provider_swarm_peer_id = *provider_swarm.local_peer_id();
        let provider_addr: Multiaddr = Protocol::Memory(u64::from(rng().random::<u16>())).into();
        provider_swarm.listen_on(provider_addr.clone()).unwrap();

        tokio::spawn(async move {
            while let Some(event) = provider_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::ProvideTipsRequest { reply_sender }) = event {
                    let _ = reply_sender.send(Bytes::new());
                    continue;
                }
                if let SwarmEvent::Behaviour(Event::ProvideBlocksRequest { reply_sender, .. }) =
                    event
                {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let _stream = reply_sender
                            .send(
                                futures::stream::iter(
                                    std::iter::repeat_with(|| Bytes::from_static(&[0; 32]))
                                        .take(blocks_count)
                                        .map(Ok),
                                )
                                .boxed(),
                            )
                            .await;
                    });
                }
            }
        });

        let mut downloader_swarm = Swarm::new_ephemeral_tokio(|_k| Behaviour::new());

        downloader_swarm.dial_and_wait(provider_addr).await;

        (downloader_swarm, provider_swarm_peer_id)
    }

    fn request_download(
        downloader_swarm: &mut Swarm<Behaviour>,
        syncs_count: usize,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: &[HeaderId],
        peer_id: PeerId,
    ) -> Vec<BlocksResponseChannel> {
        let mut channels = Vec::new();
        for _ in 0..syncs_count {
            let (tx, rx) = mpsc::channel(1);
            channels.push((tx.clone(), rx));
            downloader_swarm
                .behaviour_mut()
                .start_blocks_download(
                    peer_id,
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks.to_owned(),
                    tx,
                )
                .unwrap();
        }

        channels
    }

    fn request_tip(
        downloader_swarm: &mut Swarm<Behaviour>,
        peer_id: PeerId,
    ) -> oneshot::Receiver<TipResponse> {
        let (tx, rx) = oneshot::channel();
        downloader_swarm
            .behaviour_mut()
            .request_tip(peer_id, tx)
            .unwrap();

        rx
    }

    async fn wait_block_messages(
        expected_count: usize,
        streams: &mut Vec<BlocksResponseChannel>,
    ) -> (Vec<BlocksResponse>, Vec<BlocksResponse>) {
        let (_tx, mut receiver) = streams.pop().unwrap();
        let mut stream = receiver.recv().await.unwrap();
        let mut blocks = Vec::new();
        let mut errors = Vec::new();

        while let Some(block) = stream.next().await {
            match block {
                Ok(block) => {
                    blocks.push(block);
                    if blocks.len() == expected_count {
                        break;
                    }
                }
                Err(e) => errors.push(BlocksResponse::NetworkError(e)),
            }
        }
        (blocks, errors)
    }
}
