#![allow(
    clippy::multiple_inherent_impl,
    reason = "We spilt the impl in different blocks on purpose to ease localizing changes."
)]

use std::{
    collections::HashMap,
    error::Error,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use libp2p::{
    gossipsub::MessageId,
    identity::ed25519,
    kad::QueryId,
    swarm::{dial_opts::DialOpts, ConnectionId, DialError, SwarmEvent},
    Multiaddr, PeerId,
};
use multiaddr::{multiaddr, Protocol};
use tokio::sync::{mpsc, oneshot};

use crate::swarm::{
    behaviour::{
        gossipsub::swarm_ext::InternalBroadcastRequest, kademlia::swarm_ext::PendingQueryData,
        NomosP2pBehaviour, NomosP2pBehaviourEvent,
    },
    config::SwarmConfig,
};

/// How long to keep a connection alive once it is idling.
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(300);
// TODO: make this configurable
const BACKOFF: u64 = 5;
// TODO: make this configurable
const MAX_RETRY: usize = 3;

pub mod behaviour;
pub mod config;
pub mod protocol;

#[derive(Debug)]
pub struct DialRequest {
    pub addr: Multiaddr,
    pub result_sender: oneshot::Sender<Result<PeerId, DialError>>,
}

#[derive(Debug)]
struct InternalDialRequest {
    request: DialRequest,
    retry_count: usize,
}

impl From<DialRequest> for InternalDialRequest {
    fn from(value: DialRequest) -> Self {
        Self {
            request: value,
            retry_count: 0,
        }
    }
}

/// Wraps [`libp2p::Swarm`], and config it for use within Nomos.
pub struct Swarm {
    // A core libp2p swarm
    swarm: libp2p::Swarm<NomosP2pBehaviour>,
    pending_dials: HashMap<ConnectionId, InternalDialRequest>,
    pending_queries: HashMap<QueryId, PendingQueryData>,
    dials_channel: (
        mpsc::UnboundedSender<InternalDialRequest>,
        mpsc::UnboundedReceiver<InternalDialRequest>,
    ),
    broadcasts_channel: (
        mpsc::UnboundedSender<InternalBroadcastRequest>,
        mpsc::UnboundedReceiver<InternalBroadcastRequest>,
    ),
    pending_broadcasts: HashMap<MessageId, InternalBroadcastRequest>,
}

impl Swarm {
    /// Builds a [`Swarm`] configured for use with Nomos on top of a tokio
    /// executor.
    //
    // TODO: define error types
    pub fn build(config: SwarmConfig) -> Result<Self, Box<dyn Error>> {
        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let peer_id = PeerId::from(keypair.public());
        tracing::info!("libp2p peer_id:{}", peer_id);

        let SwarmConfig {
            gossipsub_config,
            kademlia_config,
            identify_config,
            ..
        } = config;

        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_dns()?
            .with_behaviour(|keypair| {
                NomosP2pBehaviour::new(
                    gossipsub_config,
                    kademlia_config.clone(),
                    identify_config,
                    config.protocol_name_env,
                    keypair.public(),
                )
                .unwrap()
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_CONN_TIMEOUT))
            .build();

        let listen_addr = multiaddr(config.host, config.port);
        swarm.listen_on(listen_addr.clone())?;

        // if kademlia is enabled and is not in client mode then it is operating in a
        // server mode
        if let Some(kademlia_config) = &kademlia_config {
            if !kademlia_config.client_mode {
                // libp2p2-kad server mode is implicitly enabled
                // by adding external addressess
                // <https://github.com/libp2p/rust-libp2p/blob/master/protocols/kad/CHANGELOG.md#0440>
                let external_addr = listen_addr.with(Protocol::P2p(peer_id));
                swarm.add_external_address(external_addr.clone());
                tracing::info!("Added external address: {}", external_addr);
            }
        }

        Ok(Self {
            swarm,
            pending_queries: HashMap::new(),
            dials_channel: mpsc::unbounded_channel(),
            pending_dials: HashMap::new(),
            pending_broadcasts: HashMap::new(),
            broadcasts_channel: mpsc::unbounded_channel(),
        })
    }

    pub fn start(&mut self, initial_peers: &[Multiaddr]) {
        let local_peer_id = *self.swarm.local_peer_id();
        let local_addr = self.swarm.listeners().next().cloned();

        // add local address to kademlia
        if let Some(addr) = local_addr {
            self.kademlia_add_address(local_peer_id, addr);
        }

        self.bootstrap_kad_from_peers(initial_peers);

        // We don't wait for the complete dial, but we rely on the Swarm events later on
        for initial_peer in initial_peers {
            let (tx, _) = oneshot::channel();
            let _ = self.schedule_connect(DialRequest {
                addr: initial_peer.clone(),
                result_sender: tx,
            });
        }
    }

    pub fn schedule_connect(&mut self, dial: DialRequest) -> Result<(), DialError> {
        let opt = DialOpts::from(dial.addr.clone());
        let connection_id = opt.connection_id();

        tracing::debug!(
            "attempting to dial {}. connection_id:{connection_id:?}",
            dial.addr
        );
        self.swarm.dial(opt)?;

        // Dialing has been scheduled. The result will be notified as a SwarmEvent.
        self.pending_dials.insert(connection_id, dial.into());
        Ok(())
    }

    pub fn handle_event(&mut self, event: &SwarmEvent<NomosP2pBehaviourEvent>) {
        match event {
            SwarmEvent::Behaviour(NomosP2pBehaviourEvent::Identify(identify_event)) => {
                self.handle_identify_event(identify_event);
            }

            SwarmEvent::Behaviour(NomosP2pBehaviourEvent::Kademlia(kademlia_event)) => {
                self.handle_kademlia_event(kademlia_event);
            }

            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            } => {
                tracing::debug!("connected to peer:{peer_id}, connection_id:{connection_id:?}");
                if endpoint.is_dialer() {
                    self.complete_connect(*connection_id, *peer_id);
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                cause,
                ..
            } => {
                tracing::debug!(
                    "connection closed from peer: {peer_id} {connection_id:?} due to {cause:?}"
                );
            }
            SwarmEvent::OutgoingConnectionError {
                peer_id,
                connection_id,
                error,
                ..
            } => {
                tracing::error!(
                    "Failed to connect to peer: {peer_id:?} {connection_id:?} due to: {error}"
                );
                self.retry_connect(*connection_id);
            }
            _ => {}
        }
    }

    fn complete_connect(&mut self, connection_id: ConnectionId, peer_id: PeerId) {
        let Some(dial_request) = self.pending_dials.remove(&connection_id) else {
            tracing::warn!("No pending dial found for connection {connection_id:?}");
            return;
        };
        if let Err(e) = dial_request.request.result_sender.send(Ok(peer_id)) {
            tracing::warn!("failed to send the Ok result of dialing: {e:?}");
        }
    }

    // TODO: Consider a common retry module for all use cases
    fn retry_connect(&mut self, connection_id: ConnectionId) {
        let Some(mut dial) = self.pending_dials.remove(&connection_id) else {
            tracing::warn!("No pending dial found for connection {connection_id:?}");
            return;
        };
        let Some(increased_retry_count) = dial.retry_count.checked_add(1) else {
            tracing::warn!("Reached limit of retry counts.");
            return;
        };
        if increased_retry_count > MAX_RETRY {
            tracing::debug!("Max retry({MAX_RETRY}) has been reached: {dial:?}");
            return;
        }
        dial.retry_count = increased_retry_count;

        let wait = exp_backoff(dial.retry_count);
        tracing::debug!("Retry dialing in {wait:?}: {dial:?}");

        let commands_tx = self.dials_channel.0.clone();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            schedule_connect(dial, &commands_tx);
        });
    }

    pub const fn swarm(&self) -> &libp2p::Swarm<NomosP2pBehaviour> {
        &self.swarm
    }
}

impl futures::Stream for Swarm {
    type Item = SwarmEvent<NomosP2pBehaviourEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.swarm).poll_next(cx)
    }
}

fn schedule_connect(
    dial: InternalDialRequest,
    commands_tx: &mpsc::UnboundedSender<InternalDialRequest>,
) {
    commands_tx
        .send(dial)
        .unwrap_or_else(|_| tracing::error!("could not schedule connect"));
}

#[must_use]
pub fn multiaddr(ip: std::net::Ipv4Addr, port: u16) -> Multiaddr {
    multiaddr!(Ip4(ip), Udp(port), QuicV1)
}

const fn exp_backoff(retry: usize) -> Duration {
    Duration::from_secs(BACKOFF.pow(retry as u32))
}
