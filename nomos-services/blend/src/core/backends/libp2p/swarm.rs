use core::{
    num::{NonZeroU64, NonZeroUsize},
    ops::{Deref, RangeInclusive},
};
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    time::Duration,
};

use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, Swarm, SwarmBuilder,
    swarm::{ConnectionId, dial_opts::PeerCondition},
};
use nomos_blend_message::{
    crypto::proofs::quota::inputs::prove::public::LeaderInputs,
    encap::ProofsVerifier as ProofsVerifierTrait,
};
use nomos_blend_network::core::{
    NetworkBehaviourEvent,
    with_core::behaviour::{Event as CoreToCoreEvent, IntervalStreamProvider, NegotiatedPeerState},
    with_edge::behaviour::Event as CoreToEdgeEvent,
};
use nomos_blend_scheduling::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::crypto::{
        IncomingEncapsulatedMessageWithValidatedPublicHeader,
        OutgoingEncapsulatedMessageWithValidatedPublicHeader,
    },
};
use nomos_libp2p::{DialOpts, SwarmEvent};
use rand::RngCore;
use tokio::sync::{broadcast, mpsc};

use crate::core::{
    backends::{
        PublicInfo, SessionInfo,
        libp2p::{
            LOG_TARGET, Libp2pBlendBackendSettings,
            behaviour::{BlendBehaviour, BlendBehaviourEvent},
        },
    },
    settings::BlendConfig,
};

#[derive(Debug)]
pub enum BlendSwarmMessage {
    Publish(Box<EncapsulatedMessage>),
    StartNewSession(SessionInfo<PeerId>),
    CompleteSessionTransition,
    StartNewEpoch(LeaderInputs),
    CompleteEpochTransition,
}

pub struct DialAttempt {
    /// Address of peer being dialed.
    address: Multiaddr,
    /// The latest (ongoing) attempt number.
    attempt_number: NonZeroU64,
}

#[cfg(test)]
impl DialAttempt {
    pub const fn address(&self) -> &Multiaddr {
        &self.address
    }

    pub const fn attempt_number(&self) -> NonZeroU64 {
        self.attempt_number
    }
}

pub struct BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    ProofsVerifier: ProofsVerifierTrait + 'static,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    swarm: Swarm<BlendBehaviour<ProofsVerifier, ObservationWindowProvider>>,
    swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
    incoming_message_sender:
        broadcast::Sender<IncomingEncapsulatedMessageWithValidatedPublicHeader>,
    public_info: PublicInfo<PeerId>,
    rng: Rng,
    max_dial_attempts_per_connection: NonZeroU64,
    ongoing_dials: HashMap<PeerId, DialAttempt>,
    minimum_network_size: NonZeroUsize,
}

pub struct SwarmParams<'config, Rng> {
    pub config: &'config BlendConfig<Libp2pBlendBackendSettings>,
    pub current_public_info: PublicInfo<PeerId>,
    pub rng: Rng,
    pub swarm_message_receiver: mpsc::Receiver<BlendSwarmMessage>,
    pub incoming_message_sender:
        broadcast::Sender<IncomingEncapsulatedMessageWithValidatedPublicHeader>,
    pub minimum_network_size: NonZeroUsize,
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    Rng: RngCore,
    ProofsVerifier: ProofsVerifierTrait + Clone,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + for<'c> From<(
            &'c BlendConfig<Libp2pBlendBackendSettings>,
            &'c Membership<PeerId>,
        )> + 'static,
{
    pub(super) fn new(
        SwarmParams {
            config,
            current_public_info,
            rng,
            swarm_message_receiver: swarm_messages_receiver,
            incoming_message_sender,
            minimum_network_size,
        }: SwarmParams<Rng>,
    ) -> Self {
        let keypair = config.backend.keypair();
        let listening_address = config.backend.listening_address.clone();
        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| {
                BlendBehaviour::new(
                    config,
                    current_public_info.session.membership.clone(),
                    ProofsVerifier::new(current_public_info.clone().into()),
                )
            })
            .expect("Blend Behaviour should be built")
            .with_swarm_config(|cfg| {
                // The idle timeout starts ticking once there are no active streams on a
                // connection. We want the connection to be closed as soon as
                // all streams are dropped.
                cfg.with_idle_connection_timeout(Duration::ZERO)
            })
            .build();

        swarm.listen_on(listening_address).unwrap_or_else(|e| {
            panic!("Failed to listen on Blend network: {e:?}");
        });

        let mut self_instance = Self {
            swarm,
            swarm_messages_receiver,
            incoming_message_sender,
            public_info: current_public_info,
            rng,
            max_dial_attempts_per_connection: config.backend.max_dial_attempts_per_peer,
            ongoing_dials: HashMap::with_capacity(
                *config.backend.core_peering_degree.start() as usize
            ),
            minimum_network_size,
        };

        self_instance.check_and_dial_new_peers_except(None);

        self_instance
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    Rng: RngCore,
    ProofsVerifier: ProofsVerifierTrait,
    ObservationWindowProvider:
        IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>,
{
    /// Dial random peers from the membership list,
    /// excluding the currently connected peers, the peers that we are already
    /// trying to dial, and the blocked peers.
    fn dial_random_peers_except(&mut self, amount: usize, except: Option<PeerId>) {
        let exclude_peers: HashSet<PeerId> = self
            .swarm
            .connected_peers()
            .chain(self.swarm.behaviour().blocked_peers.blocked_peers())
            .chain(self.ongoing_dials.keys())
            .chain(except.iter())
            .copied()
            .collect();
        // We need to clone else we would not be able to call `self.dial` inside which
        // requires access to `&mut self`.
        let current_membership = self.public_info.session.membership.clone();
        current_membership
            .filter_and_choose_remote_nodes(&mut self.rng, amount, &exclude_peers)
            .for_each(|peer| {
                let peer_address = peer.address.clone();
                let peer_id = peer.id;
                self.dial(peer_id, peer_address);
            });
    }

    /// Dial new peers, if necessary, to maintain the peering degree.
    /// We aim to have at least the peering degree number of "healthy" peers.
    fn check_and_dial_new_peers_except(&mut self, except: Option<PeerId>) {
        let membership_size = self.public_info.session.membership.size();
        if membership_size < self.minimum_network_size.get() {
            tracing::warn!(target: LOG_TARGET, "Not dialing any peers because set of core nodes is smaller than the minimum network size. {membership_size} < {}", self.minimum_network_size.get());
            return;
        }
        let num_new_conns_needed = self
            .minimum_healthy_peering_degree()
            .saturating_sub(self.num_healthy_peers());
        let available_connection_slots = self.available_connection_slots();
        if num_new_conns_needed > available_connection_slots {
            tracing::debug!(target: LOG_TARGET, "To maintain the minimum healthy peering degree the node would need to create {num_new_conns_needed} new connections, but only {available_connection_slots} slots are available.");
        }
        let connections_to_establish = num_new_conns_needed.min(available_connection_slots);
        self.dial_random_peers_except(connections_to_establish, except);
    }

    fn handle_disconnected_peer(&mut self, peer_id: PeerId, peer_state: NegotiatedPeerState) {
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id} disconnected with state {peer_state:?}.");
        if peer_state.is_spammy() {
            self.swarm.behaviour_mut().blocked_peers.block_peer(peer_id);
        }
        self.check_and_dial_new_peers_except(Some(peer_id));
    }

    fn handle_unhealthy_peer(&mut self, peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id} is unhealthy");
        self.check_and_dial_new_peers_except(Some(peer_id));
    }

    fn handle_blend_core_behaviour_event(&mut self, blend_event: CoreToCoreEvent) {
        match blend_event {
            nomos_blend_network::core::with_core::behaviour::Event::Message(msg, conn) => {
                // Forward message received from node to all other core nodes.
                self.forward_validated_swarm_message(&(*msg).clone().into(), conn);
                // Bubble up to service for decapsulation and delaying.
                self.report_message_to_service(*msg);
            }
            nomos_blend_network::core::with_core::behaviour::Event::UnhealthyPeer(peer_id) => {
                self.handle_unhealthy_peer(peer_id);
            }
            nomos_blend_network::core::with_core::behaviour::Event::HealthyPeer(peer_id) => {
                Self::handle_healthy_peer(peer_id);
            }
            nomos_blend_network::core::with_core::behaviour::Event::PeerDisconnected(
                peer_id,
                peer_state,
            ) => {
                self.handle_disconnected_peer(peer_id, peer_state);
            }
            nomos_blend_network::core::with_core::behaviour::Event::OutboundConnectionUpgradeFailed(peer_id) => {
                // If we ran out of dial attempts, we try to connect to another random peer that we are not yet connected to.
                if self.retry_dial(peer_id).is_some() {
                    self.dial_random_peers_except(1, Some(peer_id));
                }
            }
            nomos_blend_network::core::with_core::behaviour::Event::OutboundConnectionUpgradeSucceeded(peer_id) => {
                assert!(self.ongoing_dials.remove(&peer_id).is_some(), "Peer ID for a successfully upgraded connection must be present in storage");
            }
        }
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    ProofsVerifier: ProofsVerifierTrait,
    ObservationWindowProvider:
        IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>,
{
    /// It tries to dial the specified peer, by setting or increasing the
    /// counter of attempted dials towards the peer.
    ///
    /// This function always tries to dial and update the counter of attempted
    /// dials. Any checks about the maximum allowed dials must be performed in
    /// the context of the calling function.
    fn dial(&mut self, peer_id: PeerId, address: Multiaddr) {
        tracing::trace!(target: LOG_TARGET, "Dialing peer {peer_id:?} at address {address:?}.");
        // Set to `1` if first dial or bump to the next value if a retry.
        match self.ongoing_dials.entry(peer_id) {
            Entry::Vacant(empty_entry) => {
                empty_entry.insert(DialAttempt {
                    address: address.clone(),
                    attempt_number: 1.try_into().unwrap(),
                });
            }
            Entry::Occupied(mut existing_entry) => {
                let last_attempt_number = existing_entry.get().attempt_number;
                existing_entry.get_mut().attempt_number =
                    last_attempt_number.checked_add(1).unwrap();
            }
        }

        if let Err(e) = self.swarm.dial(
            DialOpts::peer_id(peer_id)
                .addresses(vec![address])
                // We use `Always` since we want to be able to dial a peer even if we already have
                // an established connection with it that belongs to the previous session.
                .condition(PeerCondition::Always)
                .build(),
        ) {
            tracing::error!(target: LOG_TARGET, "Failed to dial peer {peer_id:?}: {e:?}");
            self.retry_dial(peer_id);
        }
    }

    #[cfg(test)]
    pub fn dial_peer_at_addr(&mut self, peer_id: PeerId, address: Multiaddr) {
        self.dial(peer_id, address);
    }

    #[cfg(test)]
    pub const fn ongoing_dials(&self) -> &HashMap<PeerId, DialAttempt> {
        &self.ongoing_dials
    }

    /// Attempt to retry dialing the specified peer, if the maximum attempts
    /// have not already been performed.
    ///
    /// It returns `None` if a new dial attempt is performed, `Some` otherwise
    /// with the dial details of the peer that has been removed from the map
    /// of ongoing dials.
    fn retry_dial(&mut self, peer_id: PeerId) -> Option<DialAttempt> {
        let DialAttempt {
            address,
            attempt_number,
        } = self.ongoing_dials.get(&peer_id).unwrap();
        if *attempt_number < self.max_dial_attempts_per_connection {
            self.dial(peer_id, address.clone());
            return None;
        }
        tracing::trace!(target: LOG_TARGET, "Maximum attempts ({}) reached for peer {peer_id:?}. Re-dialing stopped.", self.max_dial_attempts_per_connection);
        self.ongoing_dials.remove(&peer_id)
    }

    fn publish_validated_swarm_message(
        &mut self,
        msg: &OutgoingEncapsulatedMessageWithValidatedPublicHeader,
    ) {
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .blend
            .with_core_mut()
            .publish_validated_message(msg)
        {
            tracing::error!(target: LOG_TARGET, "Failed to publish message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
        }
    }

    fn forward_validated_swarm_message(
        &mut self,
        msg: &OutgoingEncapsulatedMessageWithValidatedPublicHeader,
        except: (PeerId, ConnectionId),
    ) {
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .blend
            .with_core_mut()
            .forward_validated_message(msg, except)
        {
            tracing::error!(target: LOG_TARGET, "Failed to forward message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
        }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "Tracing macros generate more code that triggers this warning."
    )]
    fn report_message_to_service(&self, msg: IncomingEncapsulatedMessageWithValidatedPublicHeader) {
        tracing::debug!("Received message from a peer: {msg:?}");

        if let Err(e) = self.incoming_message_sender.send(msg) {
            tracing::error!(target: LOG_TARGET, "Failed to send incoming message to channel: {e}");
            tracing::info!(counter.failed_inbound_messages = 1);
        } else {
            tracing::info!(counter.successful_inbound_messages = 1);
        }
    }

    fn minimum_healthy_peering_degree(&self) -> usize {
        self.swarm
            .behaviour()
            .blend
            .with_core()
            .minimum_healthy_peering_degree()
    }

    fn num_healthy_peers(&self) -> usize {
        self.swarm.behaviour().blend.with_core().num_healthy_peers()
    }

    fn available_connection_slots(&self) -> usize {
        self.swarm
            .behaviour()
            .blend
            .with_core()
            .available_connection_slots()
    }

    fn handle_healthy_peer(peer_id: PeerId) {
        tracing::debug!(target: LOG_TARGET, "Peer {peer_id} is healthy again");
    }

    fn handle_blend_edge_behaviour_event(&mut self, blend_event: CoreToEdgeEvent) {
        match blend_event {
            nomos_blend_network::core::with_edge::behaviour::Event::Message(msg) => {
                // Forward message received from edge node to all the core nodes.
                self.publish_validated_swarm_message(&msg.clone().into());
                // Bubble up to service for decapsulation and delaying.
                self.report_message_to_service(msg);
            }
        }
    }

    fn handle_publish_swarm_message(&mut self, msg: EncapsulatedMessage) {
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .blend
            .with_core_mut()
            .validate_and_publish_message(msg)
        {
            tracing::error!(target: LOG_TARGET, "Failed to publish message to blend network: {e:?}");
            tracing::info!(counter.failed_outbound_messages = 1);
        } else {
            tracing::info!(counter.successful_outbound_messages = 1);
        }
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    ProofsVerifier: ProofsVerifierTrait + Clone,
    ObservationWindowProvider:
        IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>,
{
    fn handle_swarm_message(&mut self, msg: BlendSwarmMessage) {
        match msg {
            BlendSwarmMessage::Publish(msg) => {
                self.handle_publish_swarm_message(*msg);
            }
            BlendSwarmMessage::StartNewSession(new_session_info) => {
                self.public_info.session = new_session_info;
                self.swarm.behaviour_mut().blend.start_new_session(
                    self.public_info.session.membership.clone(),
                    ProofsVerifier::new(self.public_info.clone().into()),
                );
            }
            BlendSwarmMessage::CompleteSessionTransition => {
                self.swarm.behaviour_mut().blend.finish_session_transition();
            }
            BlendSwarmMessage::StartNewEpoch(new_epoch_public) => {
                self.public_info.epoch = new_epoch_public;
                self.swarm
                    .behaviour_mut()
                    .blend
                    .start_new_epoch(self.public_info.epoch);
            }
            BlendSwarmMessage::CompleteEpochTransition => {
                self.swarm.behaviour_mut().blend.finish_epoch_transition();
            }
        }
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    Rng: RngCore,
    ProofsVerifier: ProofsVerifierTrait + Clone,
    ObservationWindowProvider:
        IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>,
{
    fn handle_event(
        &mut self,
        event: SwarmEvent<BlendBehaviourEvent<ProofsVerifier, ObservationWindowProvider>>,
    ) {
        match event {
            SwarmEvent::Behaviour(BlendBehaviourEvent::Blend(NetworkBehaviourEvent::WithCore(
                e,
            ))) => {
                self.handle_blend_core_behaviour_event(e);
            }
            SwarmEvent::Behaviour(BlendBehaviourEvent::Blend(NetworkBehaviourEvent::WithEdge(
                e,
            ))) => {
                self.handle_blend_edge_behaviour_event(e);
            }
            // In case we fail to dial a peer, we retry. If the maximum number of trials is reached,
            // we re-evaluate the healthy connections and open a new one if needed, ignoring the
            // peer that we just failed to dial.
            SwarmEvent::OutgoingConnectionError {
                peer_id,
                connection_id,
                error,
            } => {
                tracing::error!(
                    target: LOG_TARGET,
                    "Dialing error for peer: {peer_id:?} on connection: {connection_id:?}. Error: {error:?}"
                );
                // We don't retry if `peer_id` is `None` or if we've achieved the maximum number
                // of retries for this peer.
                let is_connection_retried =
                    peer_id.is_some_and(|peer_id| self.retry_dial(peer_id).is_none());
                if !is_connection_retried {
                    self.check_and_dial_new_peers_except(peer_id);
                }
            }
            _ => {
                tracing::debug!(target: LOG_TARGET, "Received event from blend network that will be ignored.");
                tracing::info!(counter.ignored_event = 1);
            }
        }
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    Rng: RngCore,
    ProofsVerifier: ProofsVerifierTrait,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    #[cfg(test)]
    pub fn new_test<BehaviourConstructor>(
        behaviour_constructor: BehaviourConstructor,
        swarm_messages_receiver: mpsc::Receiver<BlendSwarmMessage>,
        incoming_message_sender: broadcast::Sender<
            IncomingEncapsulatedMessageWithValidatedPublicHeader,
        >,
        current_public_info: PublicInfo<PeerId>,
        rng: Rng,
        max_dial_attempts_per_connection: NonZeroU64,
        minimum_network_size: NonZeroUsize,
    ) -> Self
    where
        BehaviourConstructor: FnOnce(
            libp2p::identity::Keypair,
        )
            -> BlendBehaviour<ProofsVerifier, ObservationWindowProvider>,
    {
        use crate::test_utils::memory_test_swarm;

        Self {
            incoming_message_sender,
            public_info: current_public_info,
            max_dial_attempts_per_connection,
            ongoing_dials: HashMap::new(),
            rng,
            swarm: memory_test_swarm(Duration::from_secs(1), behaviour_constructor),
            swarm_messages_receiver,
            minimum_network_size,
        }
    }
}

impl<Rng, ProofsVerifier, ObservationWindowProvider>
    BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    Rng: RngCore,
    ProofsVerifier: ProofsVerifierTrait + Clone,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    pub(crate) async fn run(mut self) {
        loop {
            self.poll_next_internal().await;
        }
    }

    async fn poll_next_internal(&mut self) {
        self.poll_next_and_match(|_| false).await;
    }

    async fn poll_next_and_match<Predicate>(
        &mut self,
        swarm_event_match_predicate: Predicate,
    ) -> bool
    where
        Predicate:
            Fn(&SwarmEvent<BlendBehaviourEvent<ProofsVerifier, ObservationWindowProvider>>) -> bool,
    {
        tokio::select! {
            Some(msg) = self.swarm_messages_receiver.recv() => {
                self.handle_swarm_message(msg);
                false
            }
            Some(event) = self.swarm.next() => {
                let predicate_matched = swarm_event_match_predicate(&event);
                self.handle_event(event);
                predicate_matched
            }
        }
    }

    #[cfg(test)]
    pub async fn poll_next(&mut self) {
        self.poll_next_internal().await;
    }

    #[cfg(test)]
    pub async fn poll_next_until<Predicate>(&mut self, swarm_event_match_predicate: Predicate)
    where
        Predicate: Fn(&SwarmEvent<BlendBehaviourEvent<ProofsVerifier, ObservationWindowProvider>>) -> bool
            + Copy,
    {
        loop {
            if self.poll_next_and_match(swarm_event_match_predicate).await {
                break;
            }
        }
    }
}

// We implement `Deref` so we are able to call swarm methods on our own swarm.
impl<Rng, ProofsVerifier, ObservationWindowProvider> Deref
    for BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    ProofsVerifier: ProofsVerifierTrait,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    type Target = Swarm<BlendBehaviour<ProofsVerifier, ObservationWindowProvider>>;

    fn deref(&self) -> &Self::Target {
        &self.swarm
    }
}

#[cfg(test)]
// We implement `DerefMut` only for tests, since we do not want to give people a
// chance to bypass our API.
impl<Rng, ProofsVerifier, ObservationWindowProvider> core::ops::DerefMut
    for BlendSwarm<Rng, ProofsVerifier, ObservationWindowProvider>
where
    ProofsVerifier: ProofsVerifierTrait,
    ObservationWindowProvider: IntervalStreamProvider<IntervalStream: Unpin + Send, IntervalItem = RangeInclusive<u64>>
        + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.swarm
    }
}
