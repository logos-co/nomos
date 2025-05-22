use std::{
    collections::VecDeque,
    task::{Context, Poll, Waker},
};

use libp2p::{
    autonat::v2::client::{Behaviour, Config},
    swarm::{NetworkBehaviour, THandlerInEvent, ToSwarm},
    Multiaddr,
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

pub(crate) mod behaviour_ext;
pub mod swarm_ext;

type AutonatClientBehaviour = Behaviour<ChaCha20Rng>;

/// This behaviour is responsible for confirming that the addresses of the node
/// are publicly reachable.
pub struct NatBehaviour {
    /// AutoNAT client behaviour which is used to confirm if addresses of our
    /// node are indeed publicly reachable.
    autonat_client_behaviour: AutonatClientBehaviour,
    /// A queue of addresses which the caller has requested to be confirmed as
    /// publicly reachable.
    pending_external_address_candidates: VecDeque<Multiaddr>,
    /// This waker is only used when the public API of the behaviour is called.
    /// In all other cases we wake via reference in [`Self::poll`].
    waker: Option<Waker>,
}

impl NatBehaviour {
    pub fn new(config: Config) -> Self {
        let autonat_client_behaviour =
            AutonatClientBehaviour::new(ChaCha20Rng::from_entropy(), config);
        let pending_external_address_candidates = VecDeque::new();

        Self {
            autonat_client_behaviour,
            pending_external_address_candidates,
            waker: None,
        }
    }

    pub fn add_external_address_candidate(&mut self, address: Multiaddr) {
        self.pending_external_address_candidates.push_back(address);
        self.try_wake();
    }

    fn try_wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

impl NetworkBehaviour for NatBehaviour {
    type ConnectionHandler = <AutonatClientBehaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = <AutonatClientBehaviour as NetworkBehaviour>::ToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
        port_use: libp2p::core::transport::PortUse,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.autonat_client_behaviour
            .handle_established_outbound_connection(
                connection_id,
                peer,
                addr,
                role_override,
                port_use,
            )
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm) {
        self.autonat_client_behaviour.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: libp2p::PeerId,
        connection_id: libp2p::swarm::ConnectionId,
        event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        self.autonat_client_behaviour
            .on_connection_handler_event(peer_id, connection_id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        if let Poll::Ready(autonat_event) = self.autonat_client_behaviour.poll(cx) {
            return Poll::Ready(autonat_event);
        }

        if let Some(address) = self.pending_external_address_candidates.pop_front() {
            return Poll::Ready(ToSwarm::NewExternalAddrCandidate(address));
        }

        // Keep the most recent waker for `try_wake` to use.
        self.waker = Some(cx.waker().clone());

        Poll::Pending
    }
}
