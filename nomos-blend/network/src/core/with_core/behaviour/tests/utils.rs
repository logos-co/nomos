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
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, Signature, SIGNATURE_SIZE},
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

impl IntervalStreamProvider for IntervalProvider {
    type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let range = self.1.clone();
        Box::new(IntervalStream::new(interval(self.0)).map(move |_| range.clone()))
    }
}

#[derive(Default)]
pub struct IntervalProviderBuilder {
    range: Option<RangeInclusive<u64>>,
}

impl IntervalProviderBuilder {
    pub fn with_range(mut self, range: RangeInclusive<u64>) -> Self {
        self.range = Some(range);
        self
    }

    pub fn build(self) -> IntervalProvider {
        IntervalProvider(Duration::from_secs(1), self.range.unwrap_or(0..=1))
    }
}

#[derive(Default)]
pub struct BehaviourBuilder {
    provider: Option<IntervalProvider>,
    local_peer_id: Option<PeerId>,
    peering_degree: Option<RangeInclusive<usize>>,
    identity: Option<Keypair>,
}

impl BehaviourBuilder {
    pub fn with_provider(mut self, provider: IntervalProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_local_peer_id(mut self, local_peer_id: PeerId) -> Self {
        assert!(
            self.identity.is_none(),
            "Cannot set local peer ID when identity keypair is also set."
        );
        self.local_peer_id = Some(local_peer_id);
        self
    }

    pub fn with_peering_degree(mut self, peering_degree: RangeInclusive<usize>) -> Self {
        self.peering_degree = Some(peering_degree);
        self
    }

    pub fn with_identity(mut self, keypair: Keypair) -> Self {
        assert!(
            self.local_peer_id.is_none(),
            "Cannot set identity when local peer ID is also set."
        );
        self.identity = Some(keypair);
        self
    }

    pub fn build(self) -> Behaviour<IntervalProvider> {
        let local_peer_id = match (self.local_peer_id, self.identity) {
            (None, None) => PeerId::random(),
            (Some(peer_id), None) => peer_id,
            (None, Some(keypair)) => keypair.public().to_peer_id(),
            (Some(_), Some(_)) => panic!("Cannot happen."),
        };
        Behaviour {
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::new(),
            observation_window_clock_provider: self
                .provider
                .unwrap_or_else(|| IntervalProviderBuilder::default().build()),
            current_membership: None,
            peering_degree: self.peering_degree.unwrap_or(1..=1),
            local_peer_id,
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
                    ProofOfQuota::dummy(),
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
