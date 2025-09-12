use core::u64;
use std::hash::Hash;

use derivative::Derivative;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, X25519PrivateKey},
        proofs::{
            quota::ProofOfQuota,
            selection::{self, ProofOfSelection},
        },
    },
    encap::{
        self,
        decapsulated::DecapsulationOutput as InternalDecapsulationOutput,
        encapsulated::EncapsulatedMessage as InternalEncapsulatedMessage,
        unwrapped::{
            MissingProofOfSelectionVerificationInputs,
            UnwrappedEncapsulatedMessage as InternalUnwrappedEncapsulatedMessage,
        },
    },
    input::{EncapsulationInput, EncapsulationInputs as InternalEncapsulationInputs},
    Error, PayloadType,
};
use nomos_core::codec::SerdeOp;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{
    membership::Membership,
    message_blend::{ProofsGenerator as ProofsGeneratorTrait, SessionInfo},
    serde::ed25519_privkey_hex,
};

const ENCAPSULATION_COUNT: usize = 3;
pub type EncapsulatedMessage = InternalEncapsulatedMessage<ENCAPSULATION_COUNT>;
pub type EncapsulationInputs = InternalEncapsulationInputs<ENCAPSULATION_COUNT>;
pub type DecapsulationOutput = InternalDecapsulationOutput<ENCAPSULATION_COUNT>;
pub type UnwrappedEncapsulatedMessage = InternalUnwrappedEncapsulatedMessage<ENCAPSULATION_COUNT>;

/// [`CryptographicProcessor`] is responsible for wrapping and unwrapping
/// messages for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
pub struct SessionBoundCryptographicProcessor<NodeId, Rng, ProofsGenerator, ProofsVerifier> {
    settings: CryptographicProcessorSettings,
    /// The non-ephemeral encryption key (NEK) for decapsulating messages.
    non_ephemeral_encryption_key: X25519PrivateKey,
    membership: Membership<NodeId>,
    rng: Rng,
    proofs_generator: ProofsGenerator,
    proofs_verifier: ProofsVerifier,
}

#[derive(Clone, Derivative, Serialize, Deserialize)]
#[derivative(Debug)]
pub struct CryptographicProcessorSettings {
    /// The non-ephemeral signing key (NSK) corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: Ed25519PrivateKey,
    /// `ß_c`: expected number of blending operations for each locally generated
    /// message.
    pub num_blend_layers: u64,
}

impl<NodeId, Rng, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, Rng, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: ProofsGeneratorTrait,
    ProofsVerifier: encap::ProofsVerifier,
{
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        session_info: SessionInfo,
        rng: Rng,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let non_ephemeral_encryption_key = settings.non_ephemeral_signing_key.derive_x25519();
        Self {
            settings,
            non_ephemeral_encryption_key,
            membership,
            rng,
            proofs_generator: ProofsGenerator::new(session_info),
            proofs_verifier: ProofsVerifier::new(),
        }
    }
}

impl<NodeId, Rng, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, Rng, ProofsGenerator, ProofsVerifier>
where
    ProofsVerifier: encap::ProofsVerifier,
{
    pub fn decapsulate_message(
        &self,
        message: UnwrappedEncapsulatedMessage,
    ) -> Result<DecapsulationOutput, Error> {
        let Some(local_core_index) = self.membership.local_index() else {
            return Err(Error::NotCoreNodeReceiver);
        };
        message.decapsulate(
            &self.non_ephemeral_encryption_key,
            &MissingProofOfSelectionVerificationInputs {
                expected_node_index: local_core_index as u64,
                total_membership_size: self.membership.size() as u64,
            },
            self.proofs_verifier,
        )
    }
}

impl<NodeId, Rng, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, Rng, ProofsGenerator, ProofsVerifier>
where
    NodeId: Eq + Hash,
    Rng: RngCore,
    ProofsGenerator: ProofsGeneratorTrait,
{
    pub async fn encapsulate_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Cover, payload)
    }

    pub async fn encapsulate_and_serialize_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_cover_payload(payload)?.await,
        ))
    }

    pub async fn encapsulate_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Data, payload)
    }

    pub async fn encapsulate_and_serialize_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_data_payload(payload).await?,
        ))
    }

    // TODO: Think about optimizing this by, e.g., using less encapsulations if
    // there are less than 3 proofs available, or use a proof from a different pool
    // if needed (core proof for leadership message or leadership proof for
    // cover message, since the protocol does not enforce that).
    async fn encapsulate_payload(
        &mut self,
        payload_type: PayloadType,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        let (first_layer_proof, second_layer_proof, third_layer_proof) = match payload_type {
            PayloadType::Cover => (
                self.proofs_generator.get_next_core_proof().await,
                self.proofs_generator.get_next_core_proof().await,
                self.proofs_generator.get_next_core_proof().await,
            ),
            PayloadType::Data => (
                self.proofs_generator.get_next_leadership_proof().await,
                self.proofs_generator.get_next_leadership_proof().await,
                self.proofs_generator.get_next_leadership_proof().await,
            ),
        };
        let (Some(first_layer_proof), Some(second_layer_proof), Some(third_layer_proof)) =
            (first_layer_proof, second_layer_proof, third_layer_proof)
        else {
            return Err(Error::NoMoreProofOfQuotas);
        };

        let membership_size = self.membership.size();
        let (first_node_index, second_node_index, third_node_index) = (
            first_layer_proof
                .proof_of_selection
                .expected_index(membership_size)
                .expect("First node index should exist."),
            second_layer_proof
                .proof_of_selection
                .expected_index(membership_size)
                .expect("Second node index should exist."),
            third_layer_proof
                .proof_of_selection
                .expected_index(membership_size)
                .expect("Third node index should exist."),
        );
        let (first_node_signing_key, second_node_signing_key, third_node_signing_key) = (
            self.membership
                .get_node_at(first_node_index)
                .expect("First node should exist.")
                .public_key,
            self.membership
                .get_node_at(second_node_index)
                .expect("Second node should exist.")
                .public_key,
            self.membership
                .get_node_at(third_node_index)
                .expect("Third node should exist.")
                .public_key,
        );

        let inputs = EncapsulationInputs::new(
            [
                (first_layer_proof, first_node_signing_key),
                (second_layer_proof, second_node_signing_key),
                (third_layer_proof, third_node_signing_key),
            ]
            .into_iter()
            .map(|(proof, receiver_non_ephemeral_signing_key)| {
                EncapsulationInput::new(
                    proof.ephemeral_signing_key,
                    receiver_non_ephemeral_signing_key,
                    proof.proof_of_quota,
                    proof.proof_of_selection,
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        )?;

        EncapsulatedMessage::new(&inputs, payload_type, payload)
    }
}

#[must_use]
pub fn serialize_encapsulated_message(message: &EncapsulatedMessage) -> Vec<u8> {
    <EncapsulatedMessage as SerdeOp>::serialize(message)
        .expect("EncapsulatedMessage should be serializable")
        .to_vec()
}

pub fn deserialize_encapsulated_message(message: &[u8]) -> Result<EncapsulatedMessage, Error> {
    <EncapsulatedMessage as SerdeOp>::deserialize(message).map_err(|_| Error::DeserializationFailed)
}
