use core::{
    convert::Infallible,
    fmt::Debug,
    iter::repeat_with,
    ops::{Deref, DerefMut},
};

use libp2p::{
    PeerId, StreamProtocol, Swarm,
    identity::{Keypair, ed25519::PublicKey},
};
use libp2p_swarm_test::SwarmExt as _;
use nomos_blend_message::{
    PayloadType,
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey},
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{ProofOfQuota, inputs::prove::public::LeaderInputs},
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
        signatures::{SIGNATURE_SIZE, Signature},
    },
    encap::ProofsVerifier,
    input::EncapsulationInput,
};
use nomos_blend_scheduling::{EncapsulatedMessage, message_blend::crypto::EncapsulationInputs};
use nomos_core::crypto::ZkHash;
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
        *self_instance.0.public_header_mut().signature_mut() =
            Signature::from([100u8; SIGNATURE_SIZE]);
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
                    ProofOfQuota::from_bytes_unchecked([0; _]),
                    ProofOfSelection::from_bytes_unchecked([0; _]),
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

#[derive(Debug, Clone, Copy)]
pub struct AlwaysTrueVerifier;

impl ProofsVerifier for AlwaysTrueVerifier {
    type Error = Infallible;

    fn new(_public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self
    }

    fn start_epoch_transition(&mut self, _new_pol_inputs: LeaderInputs) {}

    fn complete_epoch_transition(&mut self) {}

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        use groth16::Field as _;

        Ok(ZkHash::ZERO)
    }

    fn verify_proof_of_selection(
        &self,
        _: ProofOfSelection,
        _: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
