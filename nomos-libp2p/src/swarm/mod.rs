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
    Multiaddr, PeerId,
    identity::ed25519,
    kad::QueryId,
    swarm::{ConnectionId, DialError, SwarmEvent, dial_opts::DialOpts},
};
use multiaddr::{Protocol, multiaddr};

use crate::swarm::{
    behaviour::{NomosP2pBehaviour, NomosP2pBehaviourEvent, kademlia::swarm_ext::PendingQueryData},
    config::SwarmConfig,
};

/// How long to keep a connection alive once it is idling.
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(300);

pub mod behaviour;
pub mod config;
pub mod protocol;

/// Wraps [`libp2p::Swarm`], and config it for use within Nomos.
pub struct Swarm {
    // A core libp2p swarm
    swarm: libp2p::Swarm<NomosP2pBehaviour>,
    pending_queries: HashMap<QueryId, PendingQueryData>,
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
        })
    }

    /// Initiates a connection attempt to a peer
    pub fn connect(&mut self, peer_addr: &Multiaddr) -> Result<ConnectionId, DialError> {
        let opt = DialOpts::from(peer_addr.clone());
        let connection_id = opt.connection_id();

        tracing::debug!("attempting to dial {peer_addr}. connection_id:{connection_id:?}",);
        self.swarm.dial(opt)?;
        Ok(connection_id)
    }
}

impl futures::Stream for Swarm {
    type Item = SwarmEvent<NomosP2pBehaviourEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.swarm).poll_next(cx)
    }
}

#[must_use]
pub fn multiaddr(ip: std::net::Ipv4Addr, port: u16) -> Multiaddr {
    multiaddr!(Ip4(ip), Udp(port), QuicV1)
}
