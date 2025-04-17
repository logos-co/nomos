use futures::{
    AsyncWriteExt, FutureExt,
    future::BoxFuture,
    stream::{FuturesUnordered, StreamExt},
};
use libp2p::PeerId;
use libp2p_stream::{Control, IncomingStreams};
use tokio::sync::{
    broadcast,
    mpsc::{self, UnboundedSender},
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info};

use crate::{
    behaviour::{
        BehaviourSyncEvent, IncomingSyncRequest, MAX_INCOMING_SYNCS, SYNC_PROTOCOL, SyncCommand,
        SyncError,
    },
    incoming_stream::{read_request_from_stream, send_response_to_peer},
    membership::ConnectedPeers,
    messages::SyncRequest,
    outgoing_stream::sync_after_requesting_tips,
};

pub struct SyncHandler {
    /// The local peer ID
    local_peer_id: PeerId,
    /// Control for managing streams
    control: Control,
    /// A handle to inbound streams
    incoming_streams: IncomingStreams,
    /// Streams waiting to be closed
    closing_streams: FuturesUnordered<BoxFuture<'static, ()>>,
    /// Connected peers
    connected_peers: ConnectedPeers,
    /// Sender for commands from services
    sync_commands_channel: UnboundedSender<SyncCommand>,
    /// Receiver for commands from services
    sync_commands_receiver: UnboundedReceiverStream<SyncCommand>,
    /// Read request and send behaviour event to Swarm
    read_sync_requests:
        FuturesUnordered<BoxFuture<'static, Result<IncomingSyncRequest, SyncError>>>,
    /// Futures tracking the progress of local sync operations.
    local_sync_progress: FuturesUnordered<BoxFuture<'static, Result<(), SyncError>>>,
    /// Send service responses to stream
    sending_data_to_peers: FuturesUnordered<BoxFuture<'static, Result<(), SyncError>>>,
}

impl SyncHandler {
    #[must_use]
    pub fn new(
        local_peer_id: PeerId,
        mut control: Control,
        connected_peers: ConnectedPeers,
    ) -> Self {
        let incoming_streams = control
            .accept(SYNC_PROTOCOL)
            .expect("SYNC_PROTOCOL must be valid");
        let (sync_commands_sender, sync_commands_receiver) = mpsc::unbounded_channel();
        let sync_commands_receiver = UnboundedReceiverStream::new(sync_commands_receiver);

        Self {
            local_peer_id,
            control,
            incoming_streams,
            closing_streams: FuturesUnordered::new(),
            connected_peers,
            sync_commands_channel: sync_commands_sender,
            sync_commands_receiver,
            read_sync_requests: FuturesUnordered::new(),
            local_sync_progress: FuturesUnordered::new(),
            sending_data_to_peers: FuturesUnordered::new(),
        }
    }

    pub async fn run(
        mut self,
        events_tx: broadcast::Sender<BehaviourSyncEvent>,
        mut shutdown: tokio::sync::watch::Receiver<()>,
    ) {
        info!("SyncHandler started for peer {}", self.local_peer_id);
        loop {
            tokio::select! {
                // Handle incoming streams
                maybe_stream = self.incoming_streams.next() => {
                    if let Some((peer_id, mut stream)) = maybe_stream {
                        let incoming_sync_count = self.read_sync_requests.len() + self.sending_data_to_peers.len();
                        if self.local_sync_progress.is_empty() && incoming_sync_count < MAX_INCOMING_SYNCS {
                            info!(peer_id = %peer_id, "Accepting incoming sync stream");
                            self.read_sync_requests.push(read_request_from_stream(stream));
                        } else {
                            info!(peer_id = %peer_id, "Rejecting incoming sync stream due to limits");
                            self.closing_streams.push(async move {
                                let _ = stream.close().await;
                            }.boxed());
                        }
                    }
                }

                // Read request from stream
                maybe_request = self.read_sync_requests.next(), if !self.read_sync_requests.is_empty() => {
                    if let Some(result) = maybe_request {
                        match result {
                            Ok(request) => {
                                let response_sender = request.response_sender.clone();
                                let event = match request.kind {
                                    SyncRequest::Blocks { direction } => BehaviourSyncEvent::SyncRequest {
                                        direction,
                                        response_sender,
                                    },
                                    SyncRequest::TipSlot => BehaviourSyncEvent::TipRequest {
                                        response_sender,
                                    },
                                };
                                if let Err(e) = events_tx.send(event) {
                                    error!(error = %e, "Failed to broadcast sync event");
                                }
                                self.sending_data_to_peers.push(send_response_to_peer(request));
                            }
                            Err(e) => error!(error = %e, "Failed to read sync request from stream"),
                        }
                    }
                }

                // Handle sync commands from services
                maybe_command = self.sync_commands_receiver.next() => {
                    if let Some(SyncCommand::StartSync { direction, response_sender }) = maybe_command {
                        info!(direction = ?direction, "Initiating local sync for peer {}", self.local_peer_id);
                        let local_sync = sync_after_requesting_tips(
                            self.control.clone(),
                            &self.connected_peers,
                            self.local_peer_id,
                            direction,
                            response_sender,
                        );
                        self.local_sync_progress.push(local_sync);
                    }
                }

                // Handle local sync progress
                maybe_sync = self.local_sync_progress.next(), if !self.local_sync_progress.is_empty() => {
                    if let Some(result) = maybe_sync {
                        match result {
                            Ok(()) => info!("Local sync completed successfully for peer {}", self.local_peer_id),
                            Err(e) => error!(error = %e, "Local sync failed for peer {}", self.local_peer_id),
                        }
                    }
                }

                // Handle sending data to peers
                maybe_send = self.sending_data_to_peers.next(), if !self.sending_data_to_peers.is_empty() => {
                    if let Some(result) = maybe_send {
                        match result {
                            Ok(()) => info!("Successfully sent sync response to peer"),
                            Err(e) => error!(error = %e, "Failed to send sync response to peer"),
                        }
                    }
                }

                _ = self.closing_streams.next(), if !self.closing_streams.is_empty() => {}

                _ = shutdown.changed() => {
                    info!("SyncHandler shutting down for peer {}", self.local_peer_id);
                    break;
                }
            }
        }

        info!(
            "SyncHandler run loop exited for peer {}",
            self.local_peer_id
        );
    }
    pub fn sync_commands_channel(&self) -> UnboundedSender<SyncCommand> {
        self.sync_commands_channel.clone()
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use cryptarchia_engine::Slot;
    use libp2p::{PeerId, core::Multiaddr, identity::Keypair, swarm::Swarm};
    use libp2p_stream::Behaviour as StreamBehaviour;
    use nomos_core::header::HeaderId;
    use tokio::{
        sync::{broadcast, watch, watch::Sender},
        task::JoinHandle,
        time::sleep,
    };
    use tracing::debug;

    use super::*;
    use crate::{
        behaviour::{
            BehaviourSyncEvent, BehaviourSyncReply, SyncCommand,
            test::{
                extract_peer_ids, generate_addresses, generate_keys, new_swarm_in_memory,
                perform_sync,
            },
        },
        messages::SyncDirection,
    };

    const MSG_COUNT: usize = 10;
    const NUM_SWARMS: usize = 2;

    #[tokio::test]
    async fn test_sync_forward() {
        let (request_senders, _peer_ids, handles, _sds) = setup_and_run_swarms(NUM_SWARMS);

        sleep(Duration::from_millis(200)).await;
        let blocks = perform_sync(
            request_senders[0].clone(),
            SyncDirection::Forward {
                slot: Slot::genesis(),
            },
        )
        .await;

        assert_eq!(blocks.len(), MSG_COUNT,);

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn test_sync_backward() {
        let (request_senders, peer_ids, handles, _sds) = setup_and_run_swarms(NUM_SWARMS);
        sleep(Duration::from_secs(1)).await;
        let blocks = perform_sync(
            request_senders[0].clone(),
            SyncDirection::Backward {
                start_block: HeaderId::from([0; 32]),
                peer: peer_ids[1],
            },
        )
        .await;

        assert_eq!(blocks.len(), MSG_COUNT,);

        for handle in handles {
            handle.abort();
        }
    }

    struct SwarmNetwork {
        swarms: Vec<(Swarm<StreamBehaviour>, SyncHandler)>, // Store both Swarm and SyncHandler
        swarm_command_senders: Vec<UnboundedSender<SyncCommand>>,
        swarm_addresses: Vec<Multiaddr>,
    }

    fn setup_and_run_swarms(
        num_swarms: usize,
    ) -> (
        Vec<UnboundedSender<SyncCommand>>,
        Vec<PeerId>,
        Vec<JoinHandle<()>>,
        Vec<Sender<()>>,
    ) {
        let swarm_network = setup_swarms(num_swarms);
        let peer_ids = swarm_network
            .swarms
            .iter()
            .map(|(swarm, _)| *swarm.local_peer_id())
            .collect::<Vec<_>>();

        let mut handles = Vec::new();
        let mut shutdown_handles = Vec::new();

        for (i, (swarm, sync_handler)) in swarm_network.swarms.into_iter().enumerate() {
            let addresses = swarm_network.swarm_addresses.clone();
            let (event_tx, mut event_rx) = broadcast::channel(32);
            let (shutdown_tx, shutdown_rx) = watch::channel(());
            shutdown_handles.push(shutdown_tx);

            tokio::spawn(async move { sync_handler.run(event_tx.clone(), shutdown_rx).await });

            tokio::spawn(async move {
                while let Ok(event) = event_rx.recv().await {
                    match event {
                        BehaviourSyncEvent::SyncRequest {
                            direction,
                            response_sender,
                        } => {
                            debug!(
                                "Swarm {} received sync request: direction {:?}",
                                i, direction
                            );
                            tokio::spawn(async move {
                                for _ in 0..MSG_COUNT {
                                    response_sender
                                        .send(BehaviourSyncReply::Block(vec![]))
                                        .await
                                        .expect("Failed to send block");
                                }
                            });
                        }
                        BehaviourSyncEvent::TipRequest { response_sender } => {
                            response_sender
                                .send(BehaviourSyncReply::TipSlot(Slot::from(i as u64)))
                                .await
                                .expect("Failed to send tip");
                        }
                    }
                }
            });

            let handle = tokio::spawn(async move {
                run_swarm(swarm, addresses).await;
            });

            handles.push(handle);
        }

        (
            swarm_network.swarm_command_senders,
            peer_ids,
            handles,
            shutdown_handles,
        )
    }

    fn create_swarm(
        key: &Keypair,
        peer_ids: &[PeerId],
        all_addresses: &[Multiaddr],
        index: usize,
    ) -> (
        Swarm<StreamBehaviour>,
        SyncHandler,
        UnboundedSender<SyncCommand>,
    ) {
        let behaviour = StreamBehaviour::new();
        let control = behaviour.new_control();

        let mut connected_peers = ConnectedPeers::new();
        for (i, peer) in peer_ids.iter().enumerate() {
            if i != index {
                connected_peers.add_peer(*peer, all_addresses[i].clone());
            }
        }

        let sync_handler = SyncHandler::new(peer_ids[index], control, connected_peers);
        let sender = sync_handler.sync_commands_channel();

        let mut swarm = new_swarm_in_memory(key, behaviour);
        swarm
            .listen_on(all_addresses[index].clone())
            .expect("Failed to listen");

        (swarm, sync_handler, sender)
    }

    fn setup_swarms(num_swarms: usize) -> SwarmNetwork {
        let keys = generate_keys(num_swarms);
        let peer_ids = extract_peer_ids(&keys);
        let addresses = generate_addresses(num_swarms, &peer_ids);

        let mut swarms = Vec::new();
        let mut request_senders = Vec::new();

        for (i, key) in keys.iter().enumerate().take(num_swarms) {
            let (swarm, sync_handler, sender) = create_swarm(key, &peer_ids, &addresses, i);
            swarms.push((swarm, sync_handler));
            request_senders.push(sender);
        }

        SwarmNetwork {
            swarms,
            swarm_command_senders: request_senders,
            swarm_addresses: addresses,
        }
    }

    async fn run_swarm(mut swarm: Swarm<StreamBehaviour>, addresses: Vec<Multiaddr>) {
        sleep(Duration::from_millis(100)).await;
        for address in &addresses {
            swarm.dial(address.clone()).expect("Dial failed");
        }

        loop {
            tokio::select! {
                _event = swarm.select_next_some() => {
                    {}
                }
            }
        }
    }
}
