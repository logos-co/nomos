use core::{
    fmt::Debug,
    iter::repeat_with,
    ops::{Deref, DerefMut, RangeInclusive},
    time::Duration,
};
use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures::{select, Stream, StreamExt as _};
use libp2p::{PeerId, Swarm};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::{
    crypto::{
        Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, Signature, PROOF_OF_QUOTA_SIZE,
        SIGNATURE_SIZE,
    },
    input::{EncapsulationInput, EncapsulationInputs},
    PayloadType,
};
use nomos_blend_scheduling::EncapsulatedMessage;
use nomos_libp2p::{NetworkBehaviour, SwarmEvent};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use crate::core::with_core::behaviour::{Behaviour, Event, IntervalStreamProvider};

#[derive(Clone)]
pub struct IntervalProvider(Duration);

impl IntervalProvider {
    pub fn new(interval: Duration) -> Self {
        Self(interval)
    }
}

impl Default for IntervalProvider {
    fn default() -> Self {
        Self(Duration::from_secs(1))
    }
}

impl IntervalStreamProvider for IntervalProvider {
    type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        Box::new(IntervalStream::new(interval(self.0)).map(|_| 1..=1))
    }
}

impl Default for Behaviour<IntervalProvider> {
    fn default() -> Self {
        Self {
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::new(),
            observation_window_clock_provider: IntervalProvider::default(),
            current_membership: None,
            peering_degree: 1..=2,
            local_peer_id: PeerId::random(),
        }
    }
}

impl Behaviour<IntervalProvider> {
    #[must_use]
    pub fn with_interval_provider(provider: IntervalProvider) -> Self {
        Self {
            observation_window_clock_provider: provider,
            ..Default::default()
        }
    }
}

pub struct TestSwarm<Behaviour>(Swarm<Behaviour>)
where
    Behaviour: NetworkBehaviour;

impl<Behaviour> TestSwarm<Behaviour>
where
    Behaviour: NetworkBehaviour<ToSwarm: Debug> + Send,
{
    pub fn new(behaviour: Behaviour) -> Self {
        Self(Swarm::new_ephemeral_tokio(|_| behaviour))
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
        Self(
            EncapsulatedMessage::new(&generate_valid_inputs(), PayloadType::Data, payload).unwrap(),
        )
    }

    pub fn new_with_invalid_signature(payload: &[u8]) -> Self {
        let mut self_instance = Self::new(payload);
        self_instance.0.public_header_mut().signature = Signature::from([100u8; SIGNATURE_SIZE]);
        self_instance
    }

    pub fn into_inner(self) -> EncapsulatedMessage {
        self.0
    }
}

fn generate_valid_inputs() -> EncapsulationInputs<3> {
    EncapsulationInputs::new(
        repeat_with(Ed25519PrivateKey::generate)
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

#[async_trait]
pub trait SwarmExt: libp2p_swarm_test::SwarmExt {
    async fn connect_and_wait_for_outbound_upgrade<T>(&mut self, other: &mut Swarm<T>)
    where
        T: NetworkBehaviour<ToSwarm: Debug> + Send;
}

#[async_trait]
impl SwarmExt for Swarm<Behaviour<IntervalProvider>> {
    async fn connect_and_wait_for_outbound_upgrade<T>(&mut self, other: &mut Swarm<T>)
    where
        T: NetworkBehaviour<ToSwarm: Debug> + Send,
    {
        self.connect(other).await;
        select! {
            swarm_event = self.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::OutboundConnectionUpgradeSucceeded(peer_id)) = swarm_event {
                    if peer_id == *other.local_peer_id() {
                        return;
                    }
                }
            }
            // Drive other swarm to keep polling
            _ = other.select_next_some() => {}
        }
    }
}
