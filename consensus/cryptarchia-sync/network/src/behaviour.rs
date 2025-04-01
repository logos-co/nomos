use std::task::{Context, Poll};

use futures::{
    future::BoxFuture,
    stream::{FuturesUnordered, StreamExt},
    AsyncWriteExt, FutureExt,
};
use libp2p::{
    swarm::{
        ConnectionClosed, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info};

use crate::{
    membership,
    sync_incoming::{read_request_from_stream, send_response_to_peer},
    sync_outgoing::sync_after_requesting_tips,
};

pub const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new("/nomos/cryptarchia/0.1.0/sync");

// Not sure what the right value should be. But it seems reasonable to have some
// limit
const MAX_INCOMING_SYNCS: usize = 5;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncDirection {
    Forward,
    Backward,
}

/// Sync request from a peer
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncRequest {
    Sync { direction: SyncDirection, slot: u64 },
    RequestTip,
}

/// Command from services
#[derive(Debug, Clone)]
pub enum SyncCommand {
    StartSync {
        direction: SyncDirection,
        slot: u64,
        response_sender: Sender<Vec<u8>>,
    },
}

/// Event to be sent to the Swarm
#[derive(Debug, Clone)]
pub enum BehaviourSyncEvent {
    SyncRequest {
        direction: SyncDirection,
        slot: u64,
        response_sender: Sender<BehaviourSyncReply>,
    },
    TipRequest {
        response_sender: Sender<BehaviourSyncReply>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockResponse {
    Block(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BehaviourSyncReply {
    Block(BlockResponse),
    TipData(u64),
}

#[derive(Debug)]
pub enum RequestKind {
    Sync { direction: SyncDirection, slot: u64 },
    Tip,
}

#[derive(Debug)]
pub struct IncomingSyncRequest {
    /// Incoming stream
    pub(crate) stream: Stream,
    /// Kind of request
    pub(crate) kind: RequestKind,
    /// Replies from services
    pub(crate) response_sender: Sender<BehaviourSyncReply>,
    /// Where we wait service responses
    pub(crate) response_stream: ReceiverStream<BehaviourSyncReply>,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Failed to open stream: {0}")]
    StreamOpenError(#[from] libp2p_stream::OpenStreamError),
    #[error("No peers available")]
    NoPeersAvailable,
    #[error("Channel send error: {0}")]
    ChannelSendError(#[from] mpsc::error::SendError<BehaviourSyncReply>),
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
}

pub struct SyncBehaviour<Membership> {
    /// The local peer ID
    local_peer_id: PeerId,
    /// Underlying stream behaviour
    stream_behaviour: libp2p_stream::Behaviour,
    /// Control for managing streams
    control: libp2p_stream::Control,
    /// A handle to inbound streams
    incoming_streams: libp2p_stream::IncomingStreams,
    /// Streams waiting to be closed
    closing_streams: FuturesUnordered<BoxFuture<'static, ()>>,
    /// Membership handler
    membership: Membership,
    /// Sender for commands from services
    sync_commands_sender: Sender<SyncCommand>,
    /// Receiver for commands from services
    sync_commands_receiver: ReceiverStream<SyncCommand>,
    /// Progress of local forward and backward syncs
    local_sync_progress: FuturesUnordered<BoxFuture<'static, Result<(), SyncError>>>,
    /// Read request and send behaviour event to Swarm
    read_sync_requests:
        FuturesUnordered<BoxFuture<'static, Result<IncomingSyncRequest, SyncError>>>,
    /// Send service responses to stream
    sending_data_to_peers: FuturesUnordered<BoxFuture<'static, Result<(), SyncError>>>,
}

impl<Membership> SyncBehaviour<Membership>
where
    Membership: membership::ConsensusMembershipHandler<Id = PeerId> + Clone + 'static + Send,
{
    pub fn new(local_peer_id: PeerId, membership: Membership) -> Self {
        let stream_behaviour = libp2p_stream::Behaviour::new();
        let mut control = stream_behaviour.new_control();
        let incoming_streams = control.accept(SYNC_PROTOCOL).expect("Valid protocol");

        let (sync_commands_sender, sync_commands_receiver) = mpsc::channel(10);
        let sync_commands_receiver = ReceiverStream::new(sync_commands_receiver);

        Self {
            local_peer_id,
            stream_behaviour,
            control,
            incoming_streams,
            closing_streams: FuturesUnordered::new(),
            membership,
            sync_commands_sender,
            sync_commands_receiver,
            local_sync_progress: FuturesUnordered::new(),
            read_sync_requests: FuturesUnordered::new(),
            sending_data_to_peers: FuturesUnordered::new(),
        }
    }

    pub fn sync_request_channel(&self) -> Sender<SyncCommand> {
        self.sync_commands_sender.clone()
    }

    fn handle_outgoing_syncs(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        self.process_sync_commands(cx);
        self.receive_data_from_peers(cx)
    }

    fn handle_incoming_syncs(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        self.accept_incoming_streams(cx);

        if let Some(event) = self.read_sync_requests(cx) {
            return Some(event);
        }

        self.send_data_to_peers(cx);

        // If we rejected any streams above, close them here
        while self.closing_streams.poll_next_unpin(cx) == Poll::Ready(Some(())) {}

        None
    }

    fn process_sync_commands(&mut self, cx: &mut Context<'_>) {
        while let Poll::Ready(Some(command)) = self.sync_commands_receiver.poll_next_unpin(cx) {
            match command {
                SyncCommand::StartSync {
                    direction,
                    slot,
                    response_sender,
                } => {
                    info!(
                        direction = ?direction,
                        slot = slot,
                        "Starting local sync"
                    );
                    let local_sync = sync_after_requesting_tips(
                        self.control.clone(),
                        self.membership.clone(),
                        self.local_peer_id,
                        direction,
                        slot,
                        response_sender,
                    );
                    self.local_sync_progress.push(local_sync);
                }
            }
        }
    }

    fn receive_data_from_peers(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        while let Poll::Ready(Some(result)) = self.local_sync_progress.poll_next_unpin(cx) {
            match result {
                Ok(()) => info!("Local sync completed successfully"),
                Err(e) => error!(error = %e, "Local sync failed"),
            }
        }
        None
    }

    fn accept_incoming_streams(&mut self, cx: &mut Context<'_>) {
        let incoming_sync_count = self.read_sync_requests.len() + self.sending_data_to_peers.len();

        if let Poll::Ready(Some((peer_id, mut stream))) = self.incoming_streams.poll_next_unpin(cx)
        {
            if self.local_sync_progress.is_empty() || incoming_sync_count < MAX_INCOMING_SYNCS {
                info!(peer_id = %peer_id, "Processing incoming sync stream");

                self.read_sync_requests
                    .push(read_request_from_stream(stream));
            } else {
                info!(peer_id = %peer_id, "Closing incoming sync stream");
                self.closing_streams.push(
                    async move {
                        if let Err(e) = stream.close().await {
                            error!(peer_id = %peer_id, error = %e, "Failed to close stream");
                        }
                    }
                    .boxed(),
                );
            }
        }
    }

    fn read_sync_requests(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Option<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        while let Poll::Ready(Some(result)) = self.read_sync_requests.poll_next_unpin(cx) {
            match result {
                Ok(req) => {
                    info!(kind = ?req.kind, "Incoming sync request initialized");

                    let response_sender = req.response_sender.clone();

                    let event = match req.kind {
                        RequestKind::Sync { direction, slot } => {
                            ToSwarm::GenerateEvent(BehaviourSyncEvent::SyncRequest {
                                direction,
                                slot,
                                response_sender,
                            })
                        }
                        RequestKind::Tip => {
                            ToSwarm::GenerateEvent(BehaviourSyncEvent::TipRequest {
                                response_sender,
                            })
                        }
                    };
                    self.sending_data_to_peers.push(send_response_to_peer(req));
                    return Some(event);
                }
                Err(e) => error!(error = %e, "Failed to initialize incoming request"),
            }
        }
        None
    }

    fn send_data_to_peers(&mut self, cx: &mut Context<'_>) {
        while let Poll::Ready(Some(result)) = self.sending_data_to_peers.poll_next_unpin(cx) {
            match result {
                Ok(()) => info!("Incoming sync response sending completed"),
                Err(e) => error!(error = %e, "Incoming sync response sending failed"),
            }
        }
    }
}

impl<Membership> NetworkBehaviour for SyncBehaviour<Membership>
where
    Membership: membership::ConsensusMembershipHandler<Id = PeerId> + Clone + Send + 'static,
{
    type ConnectionHandler = <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = BehaviourSyncEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.stream_behaviour
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
            .inspect(|_| {
                self.membership.add_member(peer);
                self.membership
                    .update_member_address(peer, remote_addr.clone());
            })
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        role_override: libp2p::core::Endpoint,
        port_use: libp2p::core::transport::PortUse,
    ) -> Result<THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.stream_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
            .inspect(|_| {
                self.membership.add_member(peer);
                self.membership.update_member_address(peer, addr.clone());
            })
    }
    fn on_swarm_event(&mut self, event: FromSwarm) {
        if let FromSwarm::ConnectionClosed(ConnectionClosed { peer_id, .. }) = event {
            self.membership.remove_member(&peer_id);
        }
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
    ) -> Poll<ToSwarm<<Self as NetworkBehaviour>::ToSwarm, THandlerInEvent<Self>>> {
        if let Some(event) = self.handle_outgoing_syncs(cx) {
            return Poll::Ready(event);
        }
        if let Some(event) = self.handle_incoming_syncs(cx) {
            return Poll::Ready(event);
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use libp2p::{
        core::{transport::MemoryTransport, upgrade::Version, Multiaddr},
        identity::Keypair,
        swarm::{Swarm, SwarmEvent},
        PeerId, SwarmBuilder, Transport,
    };
    use rand::Rng;
    use tokio::{
        sync::mpsc::{self, Sender},
        time::sleep,
    };
    use tracing::debug;

    use super::*;
    use crate::membership::AllNeighbours;

    const MSG_COUNT: usize = 10;

    // Use 2 at the moment. In future can run more to test parallel sync
    const NUM_SWARMS: usize = 2;

    #[tokio::test]
    async fn test_sync_forward() {
        tracing_subscriber::fmt::init();

        let (request_senders, handles) = setup_and_run_swarms(NUM_SWARMS).await;
        let blocks = perform_sync(request_senders[0].clone(), SyncDirection::Forward, 0).await;

        assert_eq!(blocks.len(), MSG_COUNT);

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn test_sync_backward() {
        tracing_subscriber::fmt::init();

        let (request_senders, handles) = setup_and_run_swarms(NUM_SWARMS).await;
        let slot = MSG_COUNT as u64;
        let blocks = perform_sync(request_senders[0].clone(), SyncDirection::Backward, slot).await;

        assert_eq!(blocks.len(), MSG_COUNT);

        for handle in handles {
            handle.abort();
        }
    }

    struct SwarmNetwork {
        swarms: Vec<Swarm<SyncBehaviour<AllNeighbours>>>,
        swarm_command_senders: Vec<Sender<SyncCommand>>,
        swarm_addresses: Vec<Multiaddr>,
    }

    async fn setup_and_run_swarms(
        num_swarms: usize,
    ) -> (Vec<Sender<SyncCommand>>, Vec<tokio::task::JoinHandle<()>>) {
        let swarm_network = setup_swarms(num_swarms);

        let mut handles = Vec::new();
        for (i, swarm) in swarm_network.swarms.into_iter().enumerate() {
            let handle = tokio::spawn(run_swarm(swarm, swarm_network.swarm_addresses.clone(), i));
            handles.push(handle);
        }

        sleep(Duration::from_millis(100)).await;
        (swarm_network.swarm_command_senders, handles)
    }

    fn generate_keys(num_swarms: usize) -> Vec<Keypair> {
        (0..num_swarms)
            .map(|_| Keypair::generate_ed25519())
            .collect()
    }

    fn extract_peer_ids(keys: &[Keypair]) -> Vec<PeerId> {
        keys.iter()
            .map(|k| PeerId::from_public_key(&k.public()))
            .collect()
    }

    fn generate_addresses(num_swarms: usize) -> Vec<Multiaddr> {
        (0..num_swarms)
            .map(|_| {
                let port = rand::thread_rng().gen::<u64>();
                format!("/memory/{port}").parse().unwrap()
            })
            .collect()
    }

    fn create_swarm(
        key: &Keypair,
        peer_id: PeerId,
        all_peer_ids: &[PeerId],
        all_addresses: &[Multiaddr],
        index: usize,
    ) -> (Swarm<SyncBehaviour<AllNeighbours>>, Sender<SyncCommand>) {
        let mut neighbours = AllNeighbours::default();

        for j in 0..all_peer_ids.len() {
            if index != j {
                neighbours.add_neighbour(all_peer_ids[j]);
                neighbours.update_address(all_peer_ids[j], all_addresses[j].clone());
            }
        }

        let behaviour = SyncBehaviour::new(peer_id, neighbours);
        let mut swarm = new_swarm_in_memory(key, behaviour);

        swarm
            .listen_on(all_addresses[index].clone())
            .expect("Failed to listen");

        let sender = swarm.behaviour().sync_request_channel();
        (swarm, sender)
    }

    fn setup_swarms(num_swarms: usize) -> SwarmNetwork {
        let keys = generate_keys(num_swarms);
        let peer_ids = extract_peer_ids(&keys);
        let addresses = generate_addresses(num_swarms);

        let mut swarms = Vec::new();
        let mut request_senders = Vec::new();

        for i in 0..num_swarms {
            let (swarm, sender) = create_swarm(&keys[i], peer_ids[i], &peer_ids, &addresses, i);
            swarms.push(swarm);
            request_senders.push(sender);
        }

        SwarmNetwork {
            swarms,
            swarm_command_senders: request_senders,
            swarm_addresses: addresses,
        }
    }
    async fn run_swarm(
        mut swarm: Swarm<SyncBehaviour<AllNeighbours>>,
        addresses: Vec<Multiaddr>,
        swarm_index: usize,
    ) {
        for address in &addresses {
            swarm.dial(address.clone()).expect("Dial failed");
        }

        loop {
            match swarm.next().await {
                Some(SwarmEvent::Behaviour(event)) => match event {
                    BehaviourSyncEvent::SyncRequest {
                        direction,
                        slot,
                        response_sender,
                    } => {
                        debug!(
                            "Swarm {} received sync request: direction {:?}, slot {}",
                            swarm_index, direction, slot
                        );
                        tokio::spawn(async move {
                            if direction == SyncDirection::Forward {
                                for _ in 0..MSG_COUNT {
                                    response_sender
                                        .send(BehaviourSyncReply::Block(BlockResponse::Block(
                                            vec![],
                                        )))
                                        .await
                                        .expect("Failed to send block");
                                }
                            } else {
                                let start = slot - MSG_COUNT as u64;
                                for _ in (start..=slot).rev() {
                                    response_sender
                                        .send(BehaviourSyncReply::Block(BlockResponse::Block(
                                            vec![],
                                        )))
                                        .await
                                        .expect("Failed to send block");
                                }
                            }
                        });
                    }
                    BehaviourSyncEvent::TipRequest { response_sender } => {
                        response_sender
                            .send(BehaviourSyncReply::TipData(swarm_index as u64 * 100))
                            .await
                            .expect("Failed to send tip");
                    }
                },
                Some(_) => continue,
                None => break,
            }
        }
    }

    async fn perform_sync(
        request_sender: Sender<SyncCommand>,
        direction: SyncDirection,
        slot: u64,
    ) -> Vec<Vec<u8>> {
        let (response_sender, mut response_receiver) = mpsc::channel(10);
        request_sender
            .send(SyncCommand::StartSync {
                direction,
                slot,
                response_sender,
            })
            .await
            .expect("Failed to send sync command");

        let mut blocks = Vec::new();
        for _ in 0..MSG_COUNT {
            if let Some(block) = response_receiver.recv().await {
                blocks.push(block);
            } else {
                break;
            }
        }
        blocks
    }

    fn new_swarm_in_memory<TBehavior>(key: &Keypair, behavior: TBehavior) -> Swarm<TBehavior>
    where
        TBehavior: NetworkBehaviour + Send,
    {
        SwarmBuilder::with_existing_identity(key.clone())
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
