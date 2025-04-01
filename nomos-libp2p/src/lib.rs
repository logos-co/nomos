mod config;

use std::{
    error::Error,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use blake2::{
    digest::{consts::U32, Digest},
    Blake2b,
};
pub use config::{secret_key_serde, SwarmConfig};
use cryptarchia_sync_network::{
    behaviour::{SyncCommand, SyncDirection},
    membership::AllNeighbours,
};
pub use libp2p::{
    self,
    core::upgrade,
    gossipsub::{self, PublishError, SubscriptionError},
    identity::{self, ed25519},
    swarm::{dial_opts::DialOpts, DialError, NetworkBehaviour, SwarmEvent},
    PeerId, SwarmBuilder, Transport,
};
use libp2p::{
    gossipsub::{Message, MessageId, TopicHash},
    swarm::ConnectionId,
};
pub use multiaddr::{multiaddr, Multiaddr, Protocol};
use tokio::sync::mpsc::{Sender, UnboundedSender};

// TODO: Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have
// this limit so large. Remove this once we transition to smaller proofs.
const DATA_LIMIT: usize = 1 << 18; // Do not serialize/deserialize more than 256 KiB

/// Wraps [`libp2p::Swarm`], and config it for use within Nomos.
pub struct Swarm {
    // A core libp2p swarm
    swarm: libp2p::Swarm<Behaviour>,
}

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    sync: cryptarchia_sync_network::behaviour::SyncBehaviour<AllNeighbours>,
}

impl Behaviour {
    fn new(
        peer_id: PeerId,
        gossipsub_config: gossipsub::Config,
        // Temporarily "static"
        membership: AllNeighbours,
    ) -> Result<Self, Box<dyn Error>> {
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Author(peer_id),
            gossipsub::ConfigBuilder::from(gossipsub_config)
                .validation_mode(gossipsub::ValidationMode::None)
                .message_id_fn(compute_message_id)
                .max_transmit_size(DATA_LIMIT)
                .build()?,
        )?;
        let sync = cryptarchia_sync_network::behaviour::SyncBehaviour::new(peer_id, membership);
        Ok(Self { gossipsub, sync })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SwarmError {
    #[error("duplicate dialing")]
    DuplicateDialing,
}

/// How long to keep a connection alive once it is idling.
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(300);
impl Swarm {
    /// Builds a [`Swarm`] configured for use with Nomos on top of a tokio
    /// executor.
    //
    // TODO: define error types
    pub fn build(config: &SwarmConfig) -> Result<Self, Box<dyn Error>> {
        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let peer_id = PeerId::from(keypair.public());
        tracing::info!("libp2p peer_id:{}", peer_id);

        // TODO: just a placeholder for now to make it compile
        let membership = AllNeighbours::new();
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_dns()?
            .with_behaviour(|_| {
                Behaviour::new(peer_id, config.gossipsub_config.clone(), membership).unwrap()
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_CONN_TIMEOUT))
            .build();

        swarm.listen_on(Self::multiaddr(config.host, config.port))?;

        Ok(Self { swarm })
    }

    /// Initiates a connection attempt to a peer
    pub fn connect(&mut self, peer_addr: &Multiaddr) -> Result<ConnectionId, DialError> {
        let opt = DialOpts::from(peer_addr.clone());
        let connection_id = opt.connection_id();

        tracing::debug!("attempting to dial {peer_addr}. connection_id:{connection_id:?}",);
        self.swarm.dial(opt)?;
        Ok(connection_id)
    }

    /// Subscribes to a topic
    ///
    /// Returns true if the topic is newly subscribed or false if already
    /// subscribed.
    pub fn subscribe(&mut self, topic: &str) -> Result<bool, SubscriptionError> {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&gossipsub::IdentTopic::new(topic))
    }

    pub fn broadcast(
        &mut self,
        topic: &str,
        message: impl Into<Vec<u8>>,
    ) -> Result<MessageId, PublishError> {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(gossipsub::IdentTopic::new(topic), message)
    }

    pub fn start_sync(
        &mut self,
        direction: SyncDirection,
        reply_channel: UnboundedSender<Vec<u8>>,
    ) {
        self.swarm
            .behaviour_mut()
            .sync
            .sync_request_channel()
            .send(SyncCommand::StartSync {
                direction,
                response_sender: reply_channel,
            })
            .expect("Failed to send sync request");
    }

    /// Unsubscribes from a topic
    ///
    /// Returns true if previously subscribed
    pub fn unsubscribe(&mut self, topic: &str) -> bool {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .unsubscribe(&gossipsub::IdentTopic::new(topic))
    }

    /// Returns a reference to the underlying [`libp2p::Swarm`]
    pub const fn swarm(&self) -> &libp2p::Swarm<Behaviour> {
        &self.swarm
    }

    pub fn is_subscribed(&mut self, topic: &str) -> bool {
        let topic_hash = Self::topic_hash(topic);

        //TODO: consider O(1) searching by having our own data structure
        self.swarm
            .behaviour_mut()
            .gossipsub
            .topics()
            .any(|h| h == &topic_hash)
    }

    #[must_use]
    pub fn topic_hash(topic: &str) -> TopicHash {
        gossipsub::IdentTopic::new(topic).hash()
    }

    #[must_use]
    pub fn multiaddr(ip: std::net::Ipv4Addr, port: u16) -> Multiaddr {
        multiaddr!(Ip4(ip), Udp(port), QuicV1)
    }
}

impl futures::Stream for Swarm {
    type Item = SwarmEvent<BehaviourEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.swarm).poll_next(cx)
    }
}

fn compute_message_id(message: &Message) -> MessageId {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(&message.data);
    MessageId::from(hasher.finalize().to_vec())
}
