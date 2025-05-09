use std::{
    error::Error,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use libp2p::{
    gossipsub::{self, MessageId, PublishError, SubscriptionError, TopicHash},
    identity::ed25519,
    kad::QueryId,
    swarm::{dial_opts::DialOpts, ConnectionId, DialError, SwarmEvent},
    Multiaddr, PeerId,
};
use multiaddr::{multiaddr, Protocol};

use crate::{Behaviour, BehaviourError, BehaviourEvent, SwarmConfig};

#[derive(thiserror::Error, Debug)]
pub enum SwarmError {
    #[error("duplicate dialing")]
    DuplicateDialing,

    #[error("no known peers")]
    NoKnownPeers,
}

/// How long to keep a connection alive once it is idling.
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(300);

/// Wraps [`libp2p::Swarm`], and config it for use within Nomos.
pub struct Swarm {
    // A core libp2p swarm
    swarm: libp2p::Swarm<Behaviour>,
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
                Behaviour::new(
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

        let listen_addr = Self::multiaddr(config.host, config.port);
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

    pub const fn swarm_mut(&mut self) -> &mut libp2p::Swarm<Behaviour> {
        &mut self.swarm
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

    pub fn get_closest_peers(
        &mut self,
        peer_id: libp2p::PeerId,
    ) -> Result<QueryId, BehaviourError> {
        self.swarm
            .behaviour_mut()
            .kademlia_get_closest_peers(peer_id)
    }

    pub fn get_kademlia_protocol_names(&self) -> Vec<String> {
        self.swarm.behaviour().get_kademlia_protocol_names()
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
