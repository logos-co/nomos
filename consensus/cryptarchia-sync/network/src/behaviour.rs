use std::{
    collections::HashMap,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, AsyncWriteExt, FutureExt, StreamExt};
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        dial_opts::DialOpts, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol,
};
use libp2p_stream::{Control, IncomingStreams};
use nomos_core::wire::packing::{pack_to_writer, unpack_from_reader};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info};

use crate::membership::ConsensusMembershipHandler;

/// The protocol used for synchronization streams in the network.
pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new("/nomos/cryptarchia/0.1.0/sync");

pub type SubnetworkId = u16;
pub type RequestId = u64;

/// Maximum number of concurrent incoming sync requests allowed.
const MAX_INCOMING_SYNCS: usize = 5;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum WireSyncCommand {
    WireForwardSyncRequest { slot: u64 },
}

impl WireSyncCommand {
    #[must_use]
    pub const fn forward_sync_request(slot: u64) -> Self {
        Self::WireForwardSyncRequest { slot }
    }
}

#[derive(Debug, Clone)]
pub enum SyncCommand {
    StartForwardSync {
        slot: u64,
        response_sender: UnboundedSender<Vec<u8>>,
    },
}

#[derive(Debug, Clone)]
pub enum BehaviourSyncingEvent {
    ForwardSyncRequest {
        slot: u64,
        response_sender: UnboundedSender<Vec<u8>>,
    },
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Failed to open stream: {0}")]
    StreamOpenError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("No peers available")]
    NoPeersAvailable,
    #[error("Channel send error")]
    ChannelSendError,
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
}

struct IncomingRequest {
    stream: Stream,
    slot: u64,
    response_sender: UnboundedSender<Vec<u8>>,
    response_stream: UnboundedReceiverStream<Vec<u8>>,
}

enum IncomingForwardSync {
    /// Initialising state, waiting for the request to be read from the stream.
    Initialising(BoxFuture<'static, Result<IncomingRequest, SyncError>>),
    /// Sending blocks state, forwarding blocks to the peer.
    SendingBlocks(BoxFuture<'static, Result<(), SyncError>>),
}

type LocalSyncFuture = BoxFuture<'static, Result<(), SyncError>>;

/// A network behaviour for synchronizing blocks across peers.
///
/// This behaviour manages both local sync requests initiated by this node and
/// incoming sync requests from other peers.
pub struct Behaviour<Membership> {
    /// The `PeerId` of the local node running this behaviour.
    local_peer_id: PeerId,
    /// The underlying libp2p stream behaviour for managing connections.
    stream_behaviour: libp2p_stream::Behaviour,
    /// Control handle for opening streams to peers.
    control: Control,
    /// Stream of incoming sync requests from other peers.
    incoming_streams: IncomingStreams,
    /// Streams waiting to be closed
    to_close_streams: HashMap<PeerId, Stream>,
    /// Membership handler providing peer information.
    membership: Membership,
    /// Channel for submitting local sync commands to this behaviour.
    sync_commands_sender: UnboundedSender<SyncCommand>,
    /// Receiver for processing local sync commands submitted via
    /// `sync_commands_sender`.
    sync_commands_receiver: UnboundedReceiverStream<SyncCommand>,
    /// Waker for scheduling future polls of this behaviour.
    waker: Option<Waker>,
    /// Future representing an in-progress local sync request, if any.
    in_progress_local_sync_request: Option<LocalSyncFuture>,
    /// Map of active incoming sync requests, keyed by request ID.
    incoming_requests: HashMap<RequestId, IncomingForwardSync>,
    /// The next ID to assign to an incoming sync request.
    next_request_id: RequestId,
}

impl<Membership> Behaviour<Membership>
where
    Membership: ConsensusMembershipHandler<Id = PeerId> + 'static,
{
    /// Creates a new `Behaviour` instance.
    ///
    /// Initializes the behaviour with a local peer ID and membership handler.
    pub fn new(local_peer_id: PeerId, membership: Membership) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut control = stream_behaviour.new_control();
        let incoming_streams = control.accept(SYNC_PROTOCOL).expect("Valid protocol");

        let (sync_commands_sender, sync_commands_receiver) = mpsc::unbounded_channel();
        let sync_commands_receiver = UnboundedReceiverStream::new(sync_commands_receiver);

        Self {
            local_peer_id,
            stream_behaviour,
            control,
            incoming_streams,
            to_close_streams: HashMap::default(),
            membership,
            sync_commands_sender,
            sync_commands_receiver,
            waker: None,
            in_progress_local_sync_request: None,
            incoming_requests: HashMap::new(),
            next_request_id: 0,
        }
    }

    /// Selects a peer to sync with.
    fn choose_peer(
        local_peer_id: PeerId,
        membership: &mut Membership,
    ) -> Result<PeerId, SyncError> {
        membership
            .members()
            .iter()
            .filter(|&id| id != &local_peer_id)
            .copied()
            .next()
            .ok_or(SyncError::NoPeersAvailable)
    }

    /// Opens a stream to a peer using the sync protocol.
    async fn open_stream(peer_id: PeerId, mut control: Control) -> Result<Stream, SyncError> {
        info!("Opening stream with peer {}", peer_id);
        control
            .open_stream(peer_id, SYNC_PROTOCOL)
            .await
            .map_err(|e| SyncError::StreamOpenError(e.to_string()))
    }

    /// Returns a channel for submitting local sync commands.
    ///
    /// This allows external code to initiate sync requests by sending
    /// `SyncCommand`s.
    pub fn sync_request_channel(&self) -> UnboundedSender<SyncCommand> {
        self.sync_commands_sender.clone()
    }

    fn poll_local_sync(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        self.try_start_local_sync(cx);
        self.poll_in_progress_local_sync(cx)
    }

    fn poll_incoming_requests(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        self.poll_new_incoming_streams(cx);
        self.poll_ongoing_incoming_requests(cx)
    }

    fn try_start_local_sync(&mut self, cx: &mut Context<'_>) {
        if self.in_progress_local_sync_request.is_none() {
            if let Poll::Ready(Some(SyncCommand::StartForwardSync {
                slot,
                response_sender,
            })) = self.sync_commands_receiver.poll_next_unpin(cx)
            {
                match Self::choose_peer(self.local_peer_id, &mut self.membership) {
                    Ok(peer_id) => {
                        info!(peer_id = %peer_id, slot = %slot, "Starting local sync");
                        let sync_future = Self::stream_blocks_from_peer(
                            peer_id,
                            &self.control,
                            slot,
                            response_sender,
                        );
                        self.in_progress_local_sync_request = Some(sync_future);
                        cx.waker().wake_by_ref();
                    }
                    Err(_) => {
                        error!("No peers available to start local sync");
                    }
                }
            }
        }
    }

    fn poll_in_progress_local_sync(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(future) = &mut self.in_progress_local_sync_request {
            match future.poll_unpin(cx) {
                Poll::Ready(Ok(())) => {
                    info!("Local sync completed");
                    self.in_progress_local_sync_request = None;
                }
                Poll::Ready(Err(e)) => {
                    error!("Local sync failed: {:?}", e);
                    self.in_progress_local_sync_request = None;
                }
                Poll::Pending => {}
            }
        }
        None
    }

    fn poll_new_incoming_streams(&mut self, cx: &mut Context<'_>) {
        if let Poll::Ready(Some((peer_id, stream))) = self.incoming_streams.poll_next_unpin(cx) {
            if self.incoming_requests.len() < MAX_INCOMING_SYNCS {
                let request_id = self.next_request_id;
                self.next_request_id += 1;

                info!(request_id = %request_id, peer_id = %peer_id, "Received new incoming stream");
                self.incoming_requests.insert(
                    request_id,
                    IncomingForwardSync::Initialising(Self::read_request_from_stream(stream)),
                );
            } else {
                info!(
                    "Maximum incoming sync requests ({}) reached, rejecting new requests",
                    MAX_INCOMING_SYNCS
                );
                self.to_close_streams.insert(peer_id, stream);
            }
        }
    }

    fn poll_ongoing_incoming_requests(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Behaviour<Membership> as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>
    {
        let mut completed_requests = Vec::new();
        let mut to_swarm_event = None;

        for (&request_id, request_state) in self.incoming_requests.iter_mut() {
            match request_state {
                IncomingForwardSync::Initialising(init_future) => {
                    match init_future.poll_unpin(cx) {
                        Poll::Ready(Ok(req)) => {
                            info!(
                                request_id = %request_id, slot = %req.slot, "Incoming request read successfully"
                            );

                            let response_sender = req.response_sender.clone();
                            let slot = req.slot;

                            *request_state = IncomingForwardSync::SendingBlocks(
                                Self::forward_blocks_to_peer(req),
                            );

                            to_swarm_event = Some(ToSwarm::GenerateEvent(
                                BehaviourSyncingEvent::ForwardSyncRequest {
                                    slot,
                                    response_sender,
                                },
                            ));
                        }
                        Poll::Ready(Err(e)) => {
                            error!(
                                request_id = %request_id, "Incoming request failed reading: {:?}", e
                            );
                            completed_requests.push(request_id);
                        }
                        Poll::Pending => {}
                    }
                }

                IncomingForwardSync::SendingBlocks(blocks_future) => {
                    match blocks_future.poll_unpin(cx) {
                        Poll::Ready(Ok(())) => {
                            info!(
                                request_id = %request_id, "Incoming sync request completed successfully"
                            );
                            completed_requests.push(request_id);
                        }
                        Poll::Ready(Err(e)) => {
                            error!(
                                request_id = %request_id, "Incoming sync failed: {:?}", e
                            );
                            completed_requests.push(request_id);
                        }
                        Poll::Pending => {}
                    }
                }
            }
        }

        for request_id in completed_requests {
            self.incoming_requests.remove(&request_id);
        }

        to_swarm_event
    }

    /// Polls for dial events from the underlying stream behaviour.
    fn poll_dial_events(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Behaviour<Membership> as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>
    {
        if let Poll::Ready(ToSwarm::Dial { mut opts }) = self.stream_behaviour.poll(cx) {
            if let Some(address) = opts
                .get_peer_id()
                .and_then(|peer_id| self.membership.get_address(&peer_id))
            {
                opts = DialOpts::peer_id(opts.get_peer_id().unwrap())
                    .addresses(vec![address])
                    .extend_addresses_through_behaviour()
                    .build();
                cx.waker().wake_by_ref();
                return Some(ToSwarm::Dial { opts });
            }
        }
        None
    }

    fn poll_close_streams(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Behaviour<Membership> as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>>
    {
        let mut to_remove = Vec::new();
        for (peer_id, stream) in self.to_close_streams.iter_mut() {
            match stream.close().poll_unpin(cx) {
                Poll::Ready(Ok(())) => {
                    info!(peer_id = %peer_id, "Closed stream");
                    to_remove.push(*peer_id);
                }
                Poll::Ready(Err(e)) => {
                    error!(peer_id = %peer_id, "Failed to close stream: {:?}", e);
                    to_remove.push(*peer_id);
                }
                Poll::Pending => {}
            }
        }

        for peer_id in to_remove {
            self.to_close_streams.remove(&peer_id);
        }

        None
    }

    /// Streams blocks from a peer.
    ///
    /// Opens a stream to the specified peer, sends a sync request for the given
    /// slot, and forwards received blocks to the provided response sender.
    fn stream_blocks_from_peer(
        peer_id: PeerId,
        control: &Control,
        slot: u64,
        response_sender: UnboundedSender<Vec<u8>>,
    ) -> BoxFuture<'static, Result<(), SyncError>> {
        let control = control.clone();
        async move {
            let mut stream = Self::open_stream(peer_id, control).await?;
            let request = WireSyncCommand::forward_sync_request(slot);
            pack_to_writer(&request, &mut stream)
                .await
                .map_err(|e| SyncError::SerializationError(e.to_string()))?;
            stream.flush().await?;

            while let Ok(block) = unpack_from_reader(&mut stream).await {
                info!(peer_id = %peer_id, "Received block");
                response_sender
                    .send(block)
                    .map_err(|_| SyncError::ChannelSendError)?;
            }
            stream.close().await?;
            Ok(())
        }
        .boxed()
    }

    /// Reads a sync request from an incoming stream.
    fn read_request_from_stream(
        mut stream: Stream,
    ) -> BoxFuture<'static, Result<IncomingRequest, SyncError>> {
        async move {
            let WireSyncCommand::WireForwardSyncRequest { slot } = unpack_from_reader(&mut stream)
                .await
                .map_err(|e| SyncError::SerializationError(e.to_string()))?;
            let (response_sender, response_receiver) = mpsc::unbounded_channel();
            Ok(IncomingRequest {
                stream,
                slot,
                response_sender,
                response_stream: UnboundedReceiverStream::new(response_receiver),
            })
        }
        .boxed()
    }

    /// Sends blocks received from a channel to a peer over a stream.
    ///
    /// Takes blocks from the response stream and writes them to the peer's
    /// stream until the response stream is exhausted.
    fn forward_blocks_to_peer(state: IncomingRequest) -> BoxFuture<'static, Result<(), SyncError>> {
        async move {
            let mut stream = state.stream;
            let mut response_stream = state.response_stream;
            while let Some(block) = response_stream.next().await {
                pack_to_writer(&block, &mut stream)
                    .await
                    .map_err(|e| SyncError::SerializationError(e.to_string()))?;
                stream.flush().await?;
            }
            stream.close().await?;
            Ok(())
        }
        .boxed()
    }
}

impl<M> NetworkBehaviour for Behaviour<M>
where
    M: ConsensusMembershipHandler<Id = PeerId> + 'static,
{
    type ConnectionHandler = <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = BehaviourSyncingEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.stream_behaviour.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: Endpoint,
        port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.stream_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.stream_behaviour
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.waker = Some(cx.waker().clone());

        if let Some(event) = self.poll_local_sync(cx) {
            return Poll::Ready(event);
        }
        if let Some(event) = self.poll_incoming_requests(cx) {
            return Poll::Ready(event);
        }
        if let Some(event) = self.poll_dial_events(cx) {
            return Poll::Ready(event);
        }
        if let Some(event) = self.poll_close_streams(cx) {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use libp2p::{
        core::{transport::MemoryTransport, upgrade::Version},
        identity::Keypair,
        swarm::{NetworkBehaviour, SwarmEvent},
        Multiaddr, PeerId, Swarm, Transport,
    };
    use rand::Rng;
    use tokio::{
        sync::mpsc,
        time::{sleep, timeout},
    };
    use tracing::debug;
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use super::*;
    use crate::{behaviour::BehaviourSyncingEvent::ForwardSyncRequest, membership::AllNeighbours};

    const MSG_COUNT: usize = 10;
    const TIMEOUT_SECS: u64 = 5;

    #[tokio::test]
    async fn test_request_blocks() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .with_writer(TestWriter::new())
            .try_init();

        let (swarm_1, swarm_2, request_sender_1) = setup_swarm_pair().await;

        let swarm_1_handle = tokio::spawn(async move { run_swarm(swarm_1).await });
        let swarm_2_handle = tokio::spawn(async move { run_swarm(swarm_2).await });

        sleep(Duration::from_secs(2)).await;

        let (response_sender_1, mut response_receiver_1) = mpsc::unbounded_channel();
        request_sender_1
            .send(SyncCommand::StartForwardSync {
                slot: 0,
                response_sender: response_sender_1,
            })
            .expect("Failed to send sync command");

        let blocks = collect_blocks(&mut response_receiver_1).await;
        assert_eq!(blocks.len(), MSG_COUNT,);

        swarm_1_handle.abort();
        swarm_2_handle.abort();
    }

    async fn setup_swarm_pair() -> (
        Swarm<Behaviour<AllNeighbours>>,
        Swarm<Behaviour<AllNeighbours>>,
        UnboundedSender<SyncCommand>,
    ) {
        let k1 = Keypair::generate_ed25519();
        let p1 = PeerId::from_public_key(&k1.public());
        let k2 = Keypair::generate_ed25519();
        let p2 = PeerId::from_public_key(&k2.public());

        let p1_id = rand::thread_rng().gen::<u64>();
        let p1_address: Multiaddr = format!("/memory/{}", p1_id).parse().unwrap();
        let p2_id = rand::thread_rng().gen::<u64>();
        let p2_address: Multiaddr = format!("/memory/{}", p2_id).parse().unwrap();

        let mut neighbours = AllNeighbours::default();
        neighbours.add_neighbour(p1);
        neighbours.add_neighbour(p2);
        neighbours.update_address(p1, p1_address.clone());
        neighbours.update_address(p2, p2_address.clone());

        let mut swarm_1 = new_swarm_in_memory(&k1, Behaviour::new(p1, neighbours.clone()));
        let mut swarm_2 = new_swarm_in_memory(&k2, Behaviour::new(p2, neighbours));
        let request_sender_1 = swarm_1.behaviour().sync_request_channel();

        swarm_1
            .listen_on(p1_address)
            .expect("Swarm 1 failed to listen");
        swarm_2
            .listen_on(p2_address)
            .expect("Swarm 2 failed to listen");

        (swarm_1, swarm_2, request_sender_1)
    }

    async fn run_swarm(mut swarm: Swarm<Behaviour<AllNeighbours>>) {
        loop {
            match swarm.next().await {
                Some(SwarmEvent::Behaviour(ForwardSyncRequest {
                    slot,
                    response_sender,
                })) => {
                    debug!("Received block request for slot {}", slot);
                    tokio::spawn(async move {
                        for _ in 0..MSG_COUNT {
                            response_sender
                                .send(vec![0; 32])
                                .expect("Failed to send block");
                        }
                    });
                }
                Some(event) => debug!("Swarm event: {:?}", event),
                None => break,
            }
        }
    }

    async fn collect_blocks(receiver: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut blocks = Vec::new();
        match timeout(Duration::from_secs(TIMEOUT_SECS), async {
            while let Some(block) = receiver.recv().await {
                debug!("Received block: {:?}", block);
                blocks.push(block);
            }
        })
        .await
        {
            Ok(()) => blocks,
            Err(_) => panic!("Timed out "),
        }
    }

    fn new_swarm_in_memory<TBehavior>(key: &Keypair, behavior: TBehavior) -> Swarm<TBehavior>
    where
        TBehavior: NetworkBehaviour + Send,
    {
        libp2p::SwarmBuilder::with_existing_identity(key.clone())
            .with_tokio()
            .with_other_transport(|_| {
                Ok(MemoryTransport::default()
                    .upgrade(Version::V1)
                    .authenticate(libp2p::plaintext::Config::new(key))
                    .multiplex(libp2p::yamux::Config::default())
                    .timeout(Duration::from_secs(20)))
            })
            .expect("Failed to build transport")
            .with_behaviour(|_| behavior)
            .expect("Failed to build behaviour")
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(20)))
            .build()
    }
}
