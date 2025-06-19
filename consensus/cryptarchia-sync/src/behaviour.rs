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
use tracing::{debug, error};

use crate::{
    downloader::Downloader,
    errors::{ChainSyncError, ChainSyncErrorKind},
    messages::{DownloadBlocksRequest, RequestMessage, SerialisedHeaderId},
    provider::{Provider, ReceivingRequestStream, MAX_ADDITIONAL_BLOCKS},
    SerialisedBlock,
};

/// Cryptarchia networking protocol for synchronizing blocks.
const SYNC_PROTOCOL_ID: &str = "/nomos/cryptarchia/sync/0.1.0";

pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new(SYNC_PROTOCOL_ID);

const MAX_INCOMING_REQUESTS: usize = 4;

type SendingBlocksRequestsFuture = BoxFuture<'static, Result<BlocksRequestStream, ChainSyncError>>;

type SendingTipRequestFuture = BoxFuture<'static, Result<TipRequestStream, ChainSyncError>>;

type ReceivingBlocksResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type ReceivingTipResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type SendingBlocksResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type SendingTipResponsesFuture = BoxFuture<'static, Result<(), ChainSyncError>>;

type ReceivingRequestsFuture = BoxFuture<'static, Result<ReceivingRequestStream, ChainSyncError>>;

type ToSwarmEvent = ToSwarm<
    <Behaviour as NetworkBehaviour>::ToSwarm,
    <<Behaviour as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
>;

pub struct BlocksRequestStream {
    pub peer_id: PeerId,
    pub stream: Libp2pStream,
    pub reply_channel: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
}

impl BlocksRequestStream {
    pub const fn new(
        peer_id: PeerId,
        stream: Stream,
        reply_channel: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
    ) -> Self {
        Self {
            peer_id,
            stream,
            reply_channel,
        }
    }
}

pub struct TipRequestStream {
    pub peer_id: PeerId,
    pub stream: Libp2pStream,
    pub reply_channel: oneshot::Sender<Result<SerialisedHeaderId, ChainSyncError>>,
}

impl TipRequestStream {
    pub const fn new(
        peer_id: PeerId,
        stream: Stream,
        reply_channel: oneshot::Sender<Result<SerialisedHeaderId, ChainSyncError>>,
    ) -> Self {
        Self {
            peer_id,
            stream,
            reply_channel,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    ProvideBlocksRequest {
        /// Return blocks up to `target_block`.
        target_block: HeaderId,
        /// The local canonical chain latest block.
        local_tip: HeaderId,
        /// The latest immutable block.
        latest_immutable_block: HeaderId,
        /// The list of additional blocks that the requester has.
        additional_blocks: HashSet<HeaderId>,
        /// Channel to send blocks to the service.
        reply_sender: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
    },
    ProvideTipsRequest {
        /// Channel to send the latest tip to the service.
        reply_sender: Sender<SerialisedHeaderId>,
    },
    DownloadBlocksResponse {
        /// The response containing a block or an error.
        result: Result<SerialisedBlock, ChainSyncError>,
    },
    GetTipResponse {
        /// The response containing the latest tip or an error.
        result: Result<SerialisedHeaderId, ChainSyncError>,
    },
}

pub struct Behaviour {
    /// The stream behavior to handle actual networking.
    stream_behaviour: StreamBehaviour,
    /// Control to open streams to peers.
    control: Control,
    /// A handle to listen to incoming stream requests.
    incoming_streams: IncomingStreams,
    /// List of connected peers.
    connected_peers: HashSet<PeerId>,
    /// Futures for reading incoming requests. This is common to both tip and
    /// block because initially we don't know which request we receive over
    /// stream. After reading the request, we use dedicated `FuturesUnordered`,
    /// either `sending_block_responses` or `sending_tip_responses`.
    receiving_requests: FuturesUnordered<ReceivingRequestsFuture>,
    /// Futures for sending block download requests. After the request is
    /// read, reading blocks are handled by `receiving_block_responses`.
    sending_block_requests: FuturesUnordered<SendingBlocksRequestsFuture>,
    /// Futures for managing the progress of locally initiated block downloads.
    receiving_block_responses: FuturesUnordered<ReceivingBlocksResponsesFuture>,
    /// Futures for managing the progress of externally initiated block
    /// downloads.
    sending_block_responses: FuturesUnordered<SendingBlocksResponsesFuture>,
    /// Futures for sending tip requests. After the request is
    /// read, the reading tip is handled by `receiving_tip_responses`.
    sending_tip_requests: FuturesUnordered<SendingTipRequestFuture>,
    /// Futures for managing the progress of locally initiated tip requests.
    receiving_tip_responses: FuturesUnordered<ReceivingTipResponsesFuture>,
    /// Futures for managing the progress of externally initiated tip
    /// requests.
    sending_tip_responses: FuturesUnordered<SendingTipResponsesFuture>,
    /// Futures for closing incoming streams that were rejected due to excess
    /// requests.
    incoming_streams_to_close: FuturesUnordered<BoxFuture<'static, ()>>,
    /// Waker to notify the behaviour when `request_tip` or
    /// `start_blocks_download` is called.
    waker: Option<std::task::Waker>,
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
            connected_peers: HashSet::new(),
            receiving_block_responses: FuturesUnordered::new(),
            sending_block_responses: FuturesUnordered::new(),
            receiving_requests: FuturesUnordered::new(),
            incoming_streams_to_close: FuturesUnordered::new(),
            sending_block_requests: FuturesUnordered::new(),
            sending_tip_requests: FuturesUnordered::new(),
            receiving_tip_responses: FuturesUnordered::new(),
            sending_tip_responses: FuturesUnordered::new(),
            waker: None,
        }
    }

    fn add_peer(&mut self, peer: PeerId) {
        self.connected_peers.insert(peer);
    }

    fn remove_peer(&mut self, peer: &PeerId) {
        self.connected_peers.remove(peer);
    }

    pub fn request_tip(
        &self,
        peer_id: PeerId,
        reply_sender: oneshot::Sender<Result<SerialisedHeaderId, ChainSyncError>>,
    ) -> Result<(), ChainSyncError> {
        if !self.connected_peers.contains(&peer_id) {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::RequestTipsError("Peer is not connected".to_owned()),
            });
        }

        let mut control = self.control.clone();

        self.sending_tip_requests.push(
            async move { Downloader::send_tip_request(peer_id, &mut control, reply_sender).await }
                .boxed(),
        );

        self.try_notify_waker();

        Ok(())
    }

    pub fn start_blocks_download(
        &self,
        peer_id: PeerId,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: HashSet<HeaderId>,
        reply_sender: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
    ) -> Result<(), ChainSyncError> {
        if !self.connected_peers.contains(&peer_id) {
            return Err(ChainSyncError {
                peer: peer_id,
                kind: ChainSyncErrorKind::StartSyncError(
                    "Peer is neither connected nor known".to_owned(),
                ),
            });
        }

        let control = self.control.clone();

        let request = DownloadBlocksRequest::new(
            target_block,
            local_tip,
            latest_immutable_block,
            additional_blocks,
        );

        self.sending_block_requests.push(
            Downloader::send_download_request(peer_id, control, request, reply_sender).boxed(),
        );

        self.try_notify_waker();

        Ok(())
    }

    fn handle_tip_request(&self, peer_id: PeerId, stream: Libp2pStream) -> Poll<ToSwarmEvent> {
        let (reply_sender, reply_receiver) = mpsc::channel(1);

        self.sending_tip_responses.push(
            async move { Provider::provide_tip(reply_receiver, peer_id, stream).await }.boxed(),
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

        let (reply_sender, reply_receiver) = mpsc::channel(1);

        self.sending_block_responses.push(
            async move { Provider::provide_blocks(reply_receiver, peer_id, stream).await }.boxed(),
        );

        Poll::Ready(ToSwarm::GenerateEvent(Event::ProvideBlocksRequest {
            target_block: request.target_block,
            local_tip: request.known_blocks.local_tip,
            latest_immutable_block: request.known_blocks.latest_immutable_block,
            additional_blocks: request.known_blocks.additional_blocks,
            reply_sender,
        }))
    }

    fn handle_incoming_stream(&self, peer_id: PeerId, mut stream: Libp2pStream) {
        let concurrent_requests = self.receiving_requests.len()
            + self.sending_block_responses.len()
            + self.sending_tip_responses.len();

        if concurrent_requests >= MAX_INCOMING_REQUESTS {
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
        }

        self.try_notify_waker();
    }

    fn handle_tip_request_available(&self, request_stream: TipRequestStream) {
        self.receiving_tip_responses
            .push(Downloader::receive_tip(request_stream).boxed());

        self.try_notify_waker();
    }

    fn handle_blocks_request_available(&self, request_stream: BlocksRequestStream) {
        self.receiving_block_responses
            .push(Downloader::receive_blocks(request_stream).boxed());

        self.try_notify_waker();
    }

    fn handle_request_ready(
        &self,
        receive_request_stream: ReceivingRequestStream,
    ) -> Poll<
        ToSwarm<
            <Self as NetworkBehaviour>::ToSwarm,
            <<Self as NetworkBehaviour>::ConnectionHandler as ConnectionHandler>::FromBehaviour,
        >,
    > {
        let (peer_id, stream, request) = receive_request_stream;

        let event = match request {
            RequestMessage::DownloadBlocksRequest(request) => {
                self.handle_download_request(peer_id, request, stream)
            }
            RequestMessage::GetTip => self.handle_tip_request(peer_id, stream),
        };

        self.try_notify_waker();

        event
    }

    fn try_notify_waker(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
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
        self.waker = Some(cx.waker().clone());

        if self.incoming_streams_to_close.poll_next_unpin(cx) == Poll::Ready(Some(())) {
            debug!("Incoming stream closed");
        }

        if let Poll::Ready(Some(result)) = self.sending_block_requests.poll_next_unpin(cx) {
            match result {
                Ok(request_stream) => {
                    self.handle_blocks_request_available(request_stream);
                }
                Err(e) => {
                    error!("Error while processing block download request: {}", e);
                }
            }

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.sending_tip_requests.poll_next_unpin(cx) {
            match result {
                Ok(request_stream) => {
                    self.handle_tip_request_available(request_stream);
                }
                Err(e) => {
                    error!("Error while processing tip request: {}", e);
                }
            }

            return Poll::Pending;
        }

        if let Poll::Ready(Some(_)) = self.receiving_block_responses.poll_next_unpin(cx) {
            return Poll::Pending;
        }

        if let Poll::Ready(Some(_)) = self.receiving_tip_responses.poll_next_unpin(cx) {
            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.sending_block_responses.poll_next_unpin(cx) {
            if let Err(e) = result {
                error!("Sending response failed: {}", e);
            }

            self.try_notify_waker();

            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.sending_tip_responses.poll_next_unpin(cx) {
            if let Err(e) = result {
                error!("Sending response failed: {}", e);
            }

            self.try_notify_waker();
            return Poll::Pending;
        }

        if let Poll::Ready(Some(result)) = self.receiving_requests.poll_next_unpin(cx) {
            match result {
                Ok(request_stream) => return self.handle_request_ready(request_stream),
                Err(e) => {
                    error!("Error while processing incoming request: {}", e);
                }
            }
        }

        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            self.handle_incoming_stream(peer_id, stream);

            return Poll::Pending;
        }

        if let Poll::Ready(ToSwarm::Dial { opts }) = self.stream_behaviour.poll(cx) {
            // If we dial, some outgoing task is created, poll again.
            self.try_notify_waker();
            return Poll::Ready(ToSwarm::Dial { opts });
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, iter, time::Duration};

    use bytes::Bytes;
    use futures::{stream::BoxStream, StreamExt as _};
    use libp2p::{swarm::SwarmEvent, Multiaddr, PeerId, Swarm};
    use libp2p_swarm_test::SwarmExt as _;
    use nomos_core::header::HeaderId;
    use rand::{rng, Rng as _};
    use tokio::sync::{mpsc, oneshot};

    use crate::{
        behaviour::{ChainSyncErrorKind, MAX_INCOMING_REQUESTS},
        messages::SerialisedHeaderId,
        provider::MAX_ADDITIONAL_BLOCKS,
        Behaviour, ChainSyncError, Event, SerialisedBlock,
    };

    #[tokio::test]
    async fn test_block_sync_between_two_swarms() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(200).await;

        let streams = request_download(
            &mut downloader_swarm,
            1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &HashSet::new(),
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (blocks, errors) = wait_block_messages(200, streams).await;

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
                HashSet::new(),
                response_tx,
            )
            .unwrap_err();

        matches!(err.kind, ChainSyncErrorKind::StartSyncError(_));
    }

    #[tokio::test]
    async fn test_reject_excess_download_requests() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(1).await;

        let streams = request_download(
            &mut downloader_swarm,
            MAX_INCOMING_REQUESTS + 1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &HashSet::new(),
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (blocks, errors) = wait_block_messages(MAX_INCOMING_REQUESTS + 1, streams).await;

        assert_eq!(blocks.len(), MAX_INCOMING_REQUESTS);
        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_reject_protocol_violation_too_many_additional_blocks() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(1).await;

        let streams = request_download(
            &mut downloader_swarm,
            MAX_INCOMING_REQUESTS + 1,
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            HeaderId::from([0; 32]),
            &iter::repeat_n(HeaderId::from([1; 32]), MAX_ADDITIONAL_BLOCKS + 1)
                .collect::<HashSet<HeaderId>>(),
            provider_peer_id,
        );

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let (_blocks, errors) = wait_block_messages(MAX_INCOMING_REQUESTS + 1, streams).await;

        assert_eq!(errors.len(), 1);
    }

    #[tokio::test]
    async fn test_get_tip() {
        let (mut downloader_swarm, provider_peer_id) = start_provider_and_downloader(0).await;

        let receiver = request_tip(&mut downloader_swarm, provider_peer_id);

        tokio::spawn(async move { downloader_swarm.loop_on_next().await });

        let tip = receiver.await.unwrap();
        assert_eq!(tip.unwrap(), Bytes::new());
    }

    async fn start_provider_and_downloader(blocks_count: usize) -> (Swarm<Behaviour>, PeerId) {
        let mut provider_swarm = new_swarm_with_quic();
        let provider_swarm_peer_id = *provider_swarm.local_peer_id();

        let provider_addr: Multiaddr = format!(
            "/ip4/127.0.0.1/udp/{}/quic-v1",
            u64::from(rng().random::<u16>())
        )
        .parse()
        .unwrap();
        provider_swarm.listen_on(provider_addr.clone()).unwrap();

        tokio::spawn(async move {
            while let Some(event) = provider_swarm.next().await {
                if let SwarmEvent::Behaviour(Event::ProvideTipsRequest { reply_sender }) = event {
                    reply_sender.send(Bytes::new()).await.unwrap();
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
                                    iter::repeat_with(|| Bytes::from_static(&[0; 32]))
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

        let mut downloader_swarm = new_swarm_with_quic();

        downloader_swarm.dial_and_wait(provider_addr).await;

        (downloader_swarm, provider_swarm_peer_id)
    }

    fn request_download(
        downloader_swarm: &mut Swarm<Behaviour>,
        syncs_count: usize,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: &HashSet<HeaderId>,
        peer_id: PeerId,
    ) -> Vec<mpsc::Receiver<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>> {
        let mut channels = Vec::new();
        for _ in 0..syncs_count {
            let (tx, rx) = mpsc::channel(1);
            channels.push(rx);
            downloader_swarm
                .behaviour_mut()
                .start_blocks_download(
                    peer_id,
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks.clone(),
                    tx,
                )
                .unwrap();
        }

        channels
    }

    fn request_tip(
        downloader_swarm: &mut Swarm<Behaviour>,
        peer_id: PeerId,
    ) -> oneshot::Receiver<Result<SerialisedHeaderId, ChainSyncError>> {
        let (tx, rx) = oneshot::channel();
        downloader_swarm
            .behaviour_mut()
            .request_tip(peer_id, tx)
            .unwrap();
        rx
    }

    async fn wait_block_messages(
        expected_count: usize,
        streams: Vec<mpsc::Receiver<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>>,
    ) -> (Vec<SerialisedBlock>, Vec<()>) {
        let mut blocks = Vec::new();
        let mut errors = Vec::new();

        for mut receiver in streams {
            let Some(mut stream) = receiver.recv().await else {
                errors.push(());
                continue;
            };

            while let Some(result) = stream.next().await {
                match result {
                    Ok(block) => {
                        blocks.push(block);
                    }
                    Err(_) => errors.push(()),
                }
            }

            if blocks.len() + errors.len() == expected_count {
                break;
            }
        }
        (blocks, errors)
    }

    fn new_swarm_with_quic() -> Swarm<Behaviour> {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_dns()
            .unwrap()
            .with_behaviour(|_| Behaviour::new())
            .unwrap()
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(10)))
            .build()
    }
}
