#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Swarm` impls into different modules for better code modularity."
)]

use std::{
    collections::HashMap,
    error::Error,
    io,
    net::Ipv4Addr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use lb_log_targets::libp2p as lb_log_targets_libp2p;
use libp2p::{
    Multiaddr, PeerId, TransportError,
    identity::ed25519,
    swarm::{ConnectionDenied, ConnectionId, DialError, SwarmEvent, dial_opts::DialOpts},
};
use multiaddr::{Protocol, multiaddr};
use rand::RngCore;

use crate::behaviour::{BehaviourConfig, GlobalPeerGate, GloballyBlockedPeer};
pub use crate::{
    SwarmConfig,
    behaviour::{Behaviour, BehaviourEvent},
};

/// How long to keep a connection alive once it is idling.
const IDLE_CONN_TIMEOUT: Duration = Duration::from_mins(5);

const LOG_TARGET: &str = lb_log_targets_libp2p::ROOT;

/// Wraps [`libp2p::Swarm`], and config it for use within Logos blockchain.
pub struct Swarm<R: Clone + Send + RngCore + 'static> {
    // A core libp2p swarm
    pub(crate) swarm: libp2p::Swarm<GlobalPeerGate<Behaviour<R>>>,
    pub(crate) global_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
    pub(crate) chain_sync_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
}

impl<R: Clone + Send + RngCore + 'static> Swarm<R> {
    /// Builds a [`Swarm`] configured for use with Logos blockchain on top of a
    /// tokio executor.
    pub fn build(config: SwarmConfig, rng: R) -> Result<Self, Box<dyn Error>> {
        Self::build_with_peer_predicates(config, HashMap::new(), rng, None, None)
    }

    /// Builds a [`Swarm`] with an optional synchronous predicate for newly
    /// opened inbound `ChainSync` streams.
    pub fn build_with_chain_sync_admission(
        config: SwarmConfig,
        rng: R,
        chain_sync_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
    ) -> Result<Self, Box<dyn Error>> {
        Self::build_with_peer_predicates(
            config,
            HashMap::new(),
            rng,
            None,
            chain_sync_peer_block_predicate,
        )
    }

    pub fn build_with_peer_predicates(
        config: SwarmConfig,
        max_data_size_by_topic: HashMap<libp2p::gossipsub::TopicHash, usize>,
        rng: R,
        global_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
        chain_sync_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
    ) -> Result<Self, Box<dyn Error>> {
        let keypair =
            libp2p::identity::Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let peer_id = PeerId::from(keypair.public());
        tracing::info!(target: LOG_TARGET, "libp2p peer_id:{}", peer_id);

        let SwarmConfig {
            gossipsub_config,
            kademlia_config,
            identify_config,
            chain_sync_config,
            nat_config,
            identify_protocol_name,
            kad_protocol_name,
            chain_sync_protocol_name,
            host,
            port,
            ..
        } = config;

        let global_predicate_for_behaviour = global_peer_block_predicate.clone();
        let chain_sync_predicate_for_behaviour = chain_sync_peer_block_predicate.clone();
        let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_dns()?
            .with_behaviour(move |keypair| {
                GlobalPeerGate::new(
                    Behaviour::new(
                        BehaviourConfig {
                            gossipsub_config,
                            kademlia_config: kademlia_config.clone(),
                            identify_config,
                            nat_config,
                            kad_protocol_name: kad_protocol_name.into(),
                            identify_protocol_name: identify_protocol_name.into(),
                            chain_sync_protocol_name: chain_sync_protocol_name.into(),
                            public_key: keypair.public(),
                            chain_sync_config,
                            max_data_size_by_topic,
                            chain_sync_peer_block_predicate: chain_sync_predicate_for_behaviour,
                        },
                        rng,
                    )
                    .expect("Behaviour should not fail to set up."),
                    global_predicate_for_behaviour.clone(),
                )
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_CONN_TIMEOUT))
            .build();

        let lb_swarm = {
            let listen_addr = multiaddr(host, port);
            let mut s = Self {
                swarm,
                global_peer_block_predicate,
                chain_sync_peer_block_predicate,
            };
            // We start listening on the provided address, which triggers the Identify flow,
            // which in turn triggers our NAT traversal state machine.
            s.start_listening_on(listen_addr.clone())
                .map_err(|e| format!("Failed to listen on {listen_addr}: {e}"))?;
            Ok::<_, Box<dyn Error>>(s)
        }?;

        Ok(lb_swarm)
    }

    /// Initiates a connection attempt to a peer
    pub fn connect(&mut self, peer_addr: &Multiaddr) -> Result<ConnectionId, DialError> {
        if let Some(peer_id) = peer_id_from_multiaddr(peer_addr)
            && self
                .global_peer_block_predicate
                .as_ref()
                .is_some_and(|predicate| predicate(peer_id))
        {
            return Err(DialError::Denied {
                cause: ConnectionDenied::new(GloballyBlockedPeer { peer_id }),
            });
        }
        let opt = DialOpts::from(peer_addr.clone());
        let connection_id = opt.connection_id();

        tracing::debug!(
            target: LOG_TARGET,
            "attempting to dial {peer_addr}. connection_id:{connection_id:?}",
        );
        self.swarm.dial(opt)?;
        Ok(connection_id)
    }

    pub fn start_listening_on(&mut self, addr: Multiaddr) -> Result<(), TransportError<io::Error>> {
        self.swarm.listen_on(addr)?;
        Ok(())
    }

    pub fn disconnect_peer(&mut self, peer_id: PeerId) -> bool {
        self.swarm.disconnect_peer_id(peer_id).is_ok()
    }

    /// Returns a reference to the underlying [`libp2p::Swarm`]
    pub const fn swarm(&self) -> &libp2p::Swarm<GlobalPeerGate<Behaviour<R>>> {
        &self.swarm
    }
}

fn peer_id_from_multiaddr(peer_addr: &Multiaddr) -> Option<PeerId> {
    peer_addr.iter().find_map(|protocol| match protocol {
        Protocol::P2p(multihash) => PeerId::from_multihash(multihash.into()).ok(),
        _ => None,
    })
}

impl<R: Clone + Send + RngCore + 'static> futures::Stream for Swarm<R> {
    type Item = SwarmEvent<BehaviourEvent<R>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.swarm).poll_next(cx)
    }
}

#[must_use]
pub fn multiaddr(ip: Ipv4Addr, port: u16) -> Multiaddr {
    multiaddr!(Ip4(ip), Udp(port), QuicV1)
}
