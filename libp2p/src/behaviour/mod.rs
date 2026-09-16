#![allow(
    clippy::multiple_inherent_impl,
    reason = "We split the `Behaviour` impls into different modules for better code modularity."
)]

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    task::{Context, Poll},
};

use lb_cryptarchia_sync::ChainSyncError;
use lb_utils::net::MAX_WIRE_MESSAGE_SIZE;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, autonat,
    core::{Endpoint, transport::PortUse},
    identify, identity, kad,
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
};
use multiaddr::Protocol;
use rand::RngCore;
use thiserror::Error;

use crate::{
    IdentifySettings, KademliaSettings, NatSettings, behaviour::gossipsub::compute_message_id,
};

pub mod chainsync;
pub mod gossipsub;
pub mod kademlia;
pub mod nat;

pub(crate) struct BehaviourConfig {
    pub gossipsub_config: libp2p::gossipsub::Config,
    pub kademlia_config: KademliaSettings,
    pub identify_config: IdentifySettings,
    pub nat_config: NatSettings,
    pub kad_protocol_name: StreamProtocol,
    pub identify_protocol_name: StreamProtocol,
    pub chain_sync_protocol_name: StreamProtocol,
    pub public_key: identity::PublicKey,
    pub chain_sync_config: lb_cryptarchia_sync::Config,
    pub max_data_size_by_topic: HashMap<libp2p::gossipsub::TopicHash, usize>,
    pub chain_sync_peer_block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
}

/// Common synchronous policy boundary for every protocol in the composite
/// behaviour.
///
/// It must not make network progress depend on an async banning-service
/// request.
pub struct GlobalPeerGate<B> {
    pub(crate) inner: B,
    block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
}

#[derive(Debug)]
pub(crate) struct GloballyBlockedPeer {
    pub(crate) peer_id: PeerId,
}

impl fmt::Display for GloballyBlockedPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "peer {} is globally blocked", self.peer_id)
    }
}

impl Error for GloballyBlockedPeer {}

impl<B> GlobalPeerGate<B> {
    pub(crate) fn new(
        inner: B,
        block_predicate: Option<lb_cryptarchia_sync::PeerBlockPredicate>,
    ) -> Self {
        Self {
            inner,
            block_predicate,
        }
    }

    fn check(&self, peer_id: PeerId) -> Result<(), ConnectionDenied> {
        if self.is_blocked(peer_id) {
            return Err(ConnectionDenied::new(GloballyBlockedPeer { peer_id }));
        }

        Ok(())
    }

    fn is_blocked(&self, peer_id: PeerId) -> bool {
        self.block_predicate
            .as_ref()
            .is_some_and(|predicate| predicate(peer_id))
    }
}

impl<B> NetworkBehaviour for GlobalPeerGate<B>
where
    B: NetworkBehaviour,
{
    type ConnectionHandler = B::ConnectionHandler;
    type ToSwarm = B::ToSwarm;

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.inner
            .handle_pending_inbound_connection(connection_id, local_addr, remote_addr)
    }

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.check(peer)?;
        self.inner.handle_established_inbound_connection(
            connection_id,
            peer,
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
        if let Some(peer) = maybe_peer {
            self.check(peer)?;
        } else if let Some(peer) = addresses.iter().find_map(peer_id_from_multiaddr) {
            self.check(peer)?;
        }
        self.inner.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
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
        self.check(peer)?;
        self.inner.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
            port_use,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        // The connection handler callback is the lowest common point at which
        // every protocol event has an authenticated peer identity.  Drop
        // handler traffic from a newly blocked peer before it can reach any
        // protocol behaviour, even while connection teardown is in progress.
        if self.is_blocked(peer_id) {
            return;
        }
        self.inner
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        loop {
            let action = match self.inner.poll(cx) {
                Poll::Ready(action) => action,
                Poll::Pending => return Poll::Pending,
            };

            let blocked = match &action {
                ToSwarm::NotifyHandler { peer_id, .. }
                | ToSwarm::NewExternalAddrOfPeer { peer_id, .. } => self.is_blocked(*peer_id),
                // This includes CloseConnection: a blocked peer must still
                // be allowed to close, otherwise teardown is prolonged.
                _ => false,
            };

            if !blocked {
                return Poll::Ready(action);
            }
        }
    }
}

fn peer_id_from_multiaddr(address: &Multiaddr) -> Option<PeerId> {
    address.iter().find_map(|protocol| match protocol {
        Protocol::P2p(multihash) => PeerId::from_multihash(multihash.into()).ok(),
        _ => None,
    })
}

#[derive(Debug, Error)]
pub enum BehaviourError {
    #[error("Operation not supported")]
    OperationNotSupported,
    #[error("Chainsync error: {0}")]
    ChainSyncError(#[from] ChainSyncError),
}

#[derive(NetworkBehaviour)]
pub struct Behaviour<Rng: Clone + Send + RngCore + 'static> {
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    // todo: support persistent store if needed
    pub(crate) kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub(crate) identify: identify::Behaviour,
    pub(crate) chain_sync: lb_cryptarchia_sync::Behaviour,
    // The spec makes it mandatory to run an autonat server for a public node.
    pub(crate) autonat_server: autonat::v2::server::Behaviour<Rng>,
    pub(crate) nat: nat::Behaviour<Rng>,
}

impl<Rng: Clone + Send + RngCore + 'static> Behaviour<Rng> {
    pub(crate) fn new(
        config: BehaviourConfig,
        rng: Rng,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let BehaviourConfig {
            gossipsub_config,
            kademlia_config,
            identify_config,
            chain_sync_config,
            nat_config,
            kad_protocol_name,
            identify_protocol_name,
            chain_sync_protocol_name,
            public_key,
            max_data_size_by_topic,
            chain_sync_peer_block_predicate,
        } = config;

        let peer_id = PeerId::from(public_key.clone());
        let gossipsub_config = gossipsub::configure_topic_size_limits(
            gossipsub_config,
            peer_id,
            max_data_size_by_topic,
        )?;

        let gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Author(peer_id),
            libp2p::gossipsub::ConfigBuilder::from(gossipsub_config)
                .validation_mode(libp2p::gossipsub::ValidationMode::None)
                .message_id_fn(compute_message_id)
                // This is only the fallback for topics without an explicit
                // application-data limit; known topics retain their overrides.
                .max_transmit_size(MAX_WIRE_MESSAGE_SIZE)
                .build()?,
        )?;

        let identify = identify::Behaviour::new(
            identify_config.to_libp2p_config(public_key, &identify_protocol_name),
        );

        let kademlia = kad::Behaviour::with_config(
            peer_id,
            kad::store::MemoryStore::new(peer_id),
            kademlia_config.to_libp2p_config(kad_protocol_name),
        );

        let autonat_server = autonat::v2::server::Behaviour::new(rng.clone());
        let nat = nat::Behaviour::new(rng, &nat_config);

        let chain_sync = lb_cryptarchia_sync::Behaviour::new_with_peer_block_predicate(
            chain_sync_protocol_name,
            chain_sync_config,
            chain_sync_peer_block_predicate,
        );

        Ok(Self {
            gossipsub,
            kademlia,
            identify,
            chain_sync,
            autonat_server,
            nat,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroUsize,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use lb_cryptarchia_sync::Config as ChainSyncConfig;
    use libp2p::{
        Transport as _, gossipsub as libp2p_gossipsub,
        swarm::{Swarm, dial_opts::DialOpts},
    };
    use rand::rngs::OsRng;

    use super::*;
    use crate::behaviour::gossipsub::configure_topic_size_limits;

    #[tokio::test]
    async fn final_behaviour_construction_preserves_topic_size_limits() {
        let node_key = identity::ed25519::SecretKey::generate();
        let keypair = identity::Keypair::from(identity::ed25519::Keypair::from(node_key));
        let public_key = keypair.public();
        let author = PeerId::from(public_key.clone());
        let topic = "test";
        let payload_size = 512;
        let topic_hash = libp2p_gossipsub::IdentTopic::new(topic).hash();
        let gossipsub_config = configure_topic_size_limits(
            libp2p_gossipsub::Config::default(),
            author,
            HashMap::from([(topic_hash.clone(), payload_size)]),
        )
        .unwrap();
        let topic_limit = gossipsub_config.max_transmit_size_for_topic(&topic_hash);

        let mut behaviour = Behaviour::<OsRng>::new(
            BehaviourConfig {
                gossipsub_config,
                kademlia_config: KademliaSettings::default(),
                identify_config: IdentifySettings::default(),
                nat_config: NatSettings::default(),
                kad_protocol_name: StreamProtocol::new("/test/kad/1.0.0"),
                identify_protocol_name: StreamProtocol::new("/test/identify/1.0.0"),
                chain_sync_protocol_name: StreamProtocol::new("/test/chainsync/1.0.0"),
                public_key,
                chain_sync_config: ChainSyncConfig {
                    peer_response_timeout: Duration::from_secs(1),
                    max_inbound_requests: NonZeroUsize::new(1).unwrap(),
                },
                max_data_size_by_topic: HashMap::new(),
                chain_sync_peer_block_predicate: None,
            },
            OsRng,
        )
        .unwrap();

        assert!(matches!(
            behaviour.gossipsub.publish(
                libp2p_gossipsub::IdentTopic::new(topic),
                vec![0; topic_limit + 1]
            ),
            Err(libp2p_gossipsub::PublishError::MessageTooLarge)
        ));
    }

    struct KnownPeerDialBehaviour {
        peer_id: PeerId,
        emitted: bool,
        dial_failures: Arc<AtomicUsize>,
        denied_failures: Arc<AtomicUsize>,
    }

    impl NetworkBehaviour for KnownPeerDialBehaviour {
        type ConnectionHandler = libp2p::swarm::dummy::ConnectionHandler;
        type ToSwarm = ();

        fn handle_established_inbound_connection(
            &mut self,
            _: ConnectionId,
            _: PeerId,
            _: &Multiaddr,
            _: &Multiaddr,
        ) -> Result<THandler<Self>, ConnectionDenied> {
            Ok(libp2p::swarm::dummy::ConnectionHandler)
        }

        fn handle_established_outbound_connection(
            &mut self,
            _: ConnectionId,
            _: PeerId,
            _: &Multiaddr,
            _: Endpoint,
            _: PortUse,
        ) -> Result<THandler<Self>, ConnectionDenied> {
            Ok(libp2p::swarm::dummy::ConnectionHandler)
        }

        fn on_swarm_event(&mut self, event: FromSwarm) {
            if let FromSwarm::DialFailure(failure) = event {
                self.dial_failures.fetch_add(1, Ordering::Relaxed);
                if matches!(failure.error, libp2p::swarm::DialError::Denied { .. }) {
                    self.denied_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        fn on_connection_handler_event(
            &mut self,
            _: PeerId,
            _: ConnectionId,
            event: THandlerOutEvent<Self>,
        ) {
            match event {}
        }

        fn poll(
            &mut self,
            _: &mut Context<'_>,
        ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
            if self.emitted {
                Poll::Pending
            } else {
                self.emitted = true;
                Poll::Ready(ToSwarm::Dial {
                    opts: DialOpts::peer_id(self.peer_id).build(),
                })
            }
        }
    }

    #[test]
    fn behavior_originated_dial_uses_swarm_denial_lifecycle() {
        let peer_id = PeerId::random();
        let predicate = Arc::new(move |candidate| candidate == peer_id);
        let dial_failures = Arc::new(AtomicUsize::new(0));
        let denied_failures = Arc::new(AtomicUsize::new(0));
        let gate = GlobalPeerGate::new(
            KnownPeerDialBehaviour {
                peer_id,
                emitted: false,
                dial_failures: Arc::clone(&dial_failures),
                denied_failures: Arc::clone(&denied_failures),
            },
            Some(predicate),
        );
        let transport = libp2p::core::transport::dummy::DummyTransport::new().boxed();
        let mut swarm = Swarm::new(
            transport,
            gate,
            PeerId::random(),
            libp2p::swarm::Config::with_tokio_executor(),
        );
        let waker = futures::task::noop_waker();
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            futures::Stream::poll_next(Pin::new(&mut swarm), &mut context),
            Poll::Pending
        ));
        assert_eq!(dial_failures.load(Ordering::Relaxed), 1);
        assert_eq!(denied_failures.load(Ordering::Relaxed), 1);
    }
}
