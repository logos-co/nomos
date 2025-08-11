use core::{
    iter::repeat_with,
    ops::{Deref, DerefMut, RangeInclusive},
    time::Duration,
};
use std::collections::{HashMap, VecDeque};

use futures::{Stream, StreamExt as _};
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use nomos_blend_message::{
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, PROOF_OF_QUOTA_SIZE},
    input::{EncapsulationInput, EncapsulationInputs},
    PayloadType,
};
use nomos_blend_scheduling::EncapsulatedMessage;
use nomos_libp2p::{NetworkBehaviour, SwarmEvent};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use crate::core::with_core::behaviour::{Behaviour, IntervalStreamProvider};

#[derive(Clone)]
pub struct IntervalProvider(Duration);

impl IntervalProvider {
    pub fn new(interval: Duration) -> Self {
        Self(interval)
    }
}

impl IntervalStreamProvider for IntervalProvider {
    type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        Box::new(IntervalStream::new(interval(self.0)).map(|_| 1..=1))
    }
}

impl Behaviour<IntervalProvider> {
    #[must_use]
    pub fn with_default_interval_provider() -> Self {
        Self {
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::new(),
            observation_window_clock_provider: IntervalProvider::new(Duration::from_secs(1)),
            current_membership: None,
            peering_degree: 1..=2,
            local_peer_id: PeerId::random(),
        }
    }

    #[must_use]
    pub fn with_interval_provider(provider: IntervalProvider) -> Self {
        Self {
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::new(),
            observation_window_clock_provider: provider,
            current_membership: None,
            peering_degree: 1..=2,
            local_peer_id: PeerId::random(),
        }
    }
}

pub struct TestSwarm<Behaviour>(Swarm<Behaviour>)
where
    Behaviour: NetworkBehaviour;

impl<Behaviour> TestSwarm<Behaviour>
where
    Behaviour: NetworkBehaviour,
{
    pub fn new(behaviour: Behaviour) -> Self {
        Self(
            SwarmBuilder::with_new_identity()
                .with_tokio()
                .with_quic()
                .with_behaviour(|_| behaviour)
                .unwrap()
                .with_swarm_config(|config| {
                    config.with_idle_connection_timeout(Duration::from_secs(1))
                })
                .build(),
        )
    }

    pub async fn start_listening(&mut self) -> Multiaddr {
        self.0
            .listen_on("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap())
            .unwrap();
        loop {
            if let SwarmEvent::NewListenAddr { address, .. } = self.0.select_next_some().await {
                break address;
            }
        }
    }
}

impl<Behaviour> Deref for TestSwarm<Behaviour>
where
    Behaviour: NetworkBehaviour,
{
    type Target = Swarm<Behaviour>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<Behaviour> DerefMut for TestSwarm<Behaviour>
where
    Behaviour: NetworkBehaviour,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct TestEncapsulatedMessage(EncapsulatedMessage);

impl TestEncapsulatedMessage {
    pub fn new(payload: &[u8]) -> Self {
        Self(EncapsulatedMessage::new(&generate_inputs(), PayloadType::Data, payload).unwrap())
    }

    pub fn into_inner(self) -> EncapsulatedMessage {
        self.0
    }
}

fn generate_inputs() -> EncapsulationInputs<3> {
    EncapsulationInputs::new(
        repeat_with(|| Ed25519PrivateKey::generate())
            .take(3)
            .map(|recipient_signing_key| {
                let recipient_signing_pubkey = recipient_signing_key.public_key();
                EncapsulationInput::new(
                    Ed25519PrivateKey::generate(),
                    &recipient_signing_pubkey,
                    ProofOfQuota::from([0u8; PROOF_OF_QUOTA_SIZE]),
                    ProofOfSelection::dummy(),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
    .unwrap()
}

impl Deref for TestEncapsulatedMessage {
    type Target = EncapsulatedMessage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TestEncapsulatedMessage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
