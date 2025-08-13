use core::{
    fmt::Debug,
    iter::repeat_with,
    ops::{Deref, DerefMut, RangeInclusive},
    time::Duration,
};
use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures::{select, Stream, StreamExt as _};
use libp2p::{
    identity::{ed25519::PublicKey, Keypair},
    PeerId, Swarm,
};
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
pub struct IntervalProvider(Duration, RangeInclusive<u64>);

impl IntervalProvider {
    pub fn new(interval: Duration, range: RangeInclusive<u64>) -> Self {
        Self(interval, range)
    }

    pub fn with_range(range: RangeInclusive<u64>) -> Self {
        Self(Self::default().0, range)
    }

    pub fn with_interval(interval: Duration) -> Self {
        Self(interval, Self::default().1)
    }
}

impl Default for IntervalProvider {
    fn default() -> Self {
        Self(Duration::from_secs(1), 0..=1)
    }
}

impl IntervalStreamProvider for IntervalProvider {
    type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let range = self.1.clone();
        Box::new(IntervalStream::new(interval(self.0)).map(move |_| range.clone()))
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
            peering_degree: 1..=1,
            local_peer_id: PeerId::random(),
        }
    }
}

impl Behaviour<IntervalProvider> {
    #[must_use]
    pub fn with_local_peer_id(local_peer_id: PeerId) -> Self {
        Self {
            local_peer_id,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn with_local_peer_id_and_peering_degree(
        local_peer_id: PeerId,
        peering_degree: RangeInclusive<usize>,
    ) -> Self {
        Self {
            peering_degree,
            local_peer_id,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn with_identity(identity: &Keypair) -> Self {
        Self {
            local_peer_id: identity.public().to_peer_id(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn with_identity_and_provider(identity: &Keypair, provider: IntervalProvider) -> Self {
        let mut self_instance = Self::with_identity(identity);
        self_instance.observation_window_clock_provider = provider;

        self_instance
    }
}

pub struct TestSwarm<Behaviour>(Swarm<Behaviour>)
where
    Behaviour: NetworkBehaviour;

impl<Behaviour> TestSwarm<Behaviour>
where
    Behaviour: NetworkBehaviour<ToSwarm: Debug> + Send,
{
    pub fn new<BehaviourConstructor>(behaviour_fn: BehaviourConstructor) -> Self
    where
        BehaviourConstructor: FnOnce(Keypair) -> Behaviour,
    {
        Self(Swarm::new_ephemeral_tokio(behaviour_fn))
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

/// Our test swarm generates random ed25519 identities. Hence, using `0`
/// guarantees us that this value will always be smaller than the random
/// identities.
pub fn smallest_peer_id() -> PeerId {
    PeerId::from_public_key(&PublicKey::try_from_bytes(&[0u8; 32]).unwrap().into())
}

/// Our test swarm generates random ed25519 identities. Hence, using `255`
/// guarantees us that this value will always be larger than the random
/// identities.
pub fn largest_peer_id() -> PeerId {
    PeerId::from_public_key(&PublicKey::try_from_bytes(&[255u8; 32]).unwrap().into())
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
