#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Swarm` impls into different modules for better code modularity."
)]

use std::{
    error::Error,
    net::Ipv4Addr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use libp2p::{
    identity::ed25519,
    swarm::{dial_opts::DialOpts, ConnectionId, DialError, SwarmEvent},
    Multiaddr, PeerId,
};
use multiaddr::{multiaddr, Protocol};

use crate::{
    behaviour::{Behaviour, BehaviourEvent},
    SwarmConfig,
};

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
    pub(crate) swarm: libp2p::Swarm<Behaviour>,
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
                    &kademlia_config,
                    &identify_config,
                    config.protocol_name_env,
                    keypair.public(),
                )
                .expect("Behaviour should not fail to set up.")
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_CONN_TIMEOUT))
            .build();

        let listen_addr = multiaddr(config.host, config.port);
        swarm.listen_on(listen_addr.clone())?;

        // if kademlia is not in client mode then it is operating in a
        // server mode
        if !kademlia_config.client_mode {
            // libp2p2-kad server mode is implicitly enabled
            // by adding external addressess
            // <https://github.com/libp2p/rust-libp2p/blob/master/protocols/kad/CHANGELOG.md#0440>
            let external_addr = listen_addr.with(Protocol::P2p(peer_id));
            swarm.add_external_address(external_addr.clone());
            tracing::info!("Added external address: {}", external_addr);
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

    /// Returns a reference to the underlying [`libp2p::Swarm`]
    pub const fn swarm(&self) -> &libp2p::Swarm<Behaviour> {
        &self.swarm
    }
}

impl futures::Stream for Swarm {
    type Item = SwarmEvent<BehaviourEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.swarm).poll_next(cx)
    }
}

#[must_use]
pub fn multiaddr(ip: Ipv4Addr, port: u16) -> Multiaddr {
    multiaddr!(Ip4(ip), Udp(port), QuicV1)
}
