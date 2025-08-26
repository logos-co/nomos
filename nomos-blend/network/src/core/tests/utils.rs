use core::{
    fmt::Debug,
    iter::repeat_with,
    ops::{Deref, DerefMut},
};

use libp2p::{
    identity::{ed25519::PublicKey, Keypair},
    PeerId, StreamProtocol, Swarm,
};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::{
    crypto::{Ed25519PrivateKey, ProofOfQuota, ProofOfSelection, Signature, SIGNATURE_SIZE},
    input::EncapsulationInput,
    PayloadType,
};
use nomos_blend_scheduling::{message_blend::crypto::EncapsulationInputs, EncapsulatedMessage};
use nomos_libp2p::NetworkBehaviour;

pub const PROTOCOL_NAME: StreamProtocol = StreamProtocol::new("/blend/core-behaviour/test");

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

fn generate_valid_inputs() -> EncapsulationInputs {
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
