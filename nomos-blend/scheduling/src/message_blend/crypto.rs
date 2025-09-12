use std::hash::Hash;

use derivative::Derivative;
use nomos_blend_message::{
    crypto::keys::{Ed25519PrivateKey, X25519PrivateKey},
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
pub struct SessionBoundCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier> {
    settings: CryptographicProcessorSettings,
    /// The non-ephemeral encryption key (NEK) for decapsulating messages.
    non_ephemeral_encryption_key: X25519PrivateKey,
    membership: Membership<NodeId>,
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

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: ProofsGeneratorTrait,
    ProofsVerifier: encap::ProofsVerifier,
{
    #[must_use]
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        session_info: SessionInfo,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let non_ephemeral_encryption_key = settings.non_ephemeral_signing_key.derive_x25519();
        Self {
            settings,
            non_ephemeral_encryption_key,
            membership,
            proofs_generator: ProofsGenerator::new(session_info),
            proofs_verifier: ProofsVerifier::new(),
        }
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
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
            &self.proofs_verifier,
        )
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SessionBoundCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    NodeId: Eq + Hash,
    ProofsGenerator: ProofsGeneratorTrait,
{
    pub async fn encapsulate_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Cover, payload).await
    }

    pub async fn encapsulate_and_serialize_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_cover_payload(payload).await?,
        ))
    }

    pub async fn encapsulate_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Data, payload).await
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
        let mut proofs = Vec::with_capacity(self.settings.num_blend_layers as usize);

        for _ in 0..self.settings.num_blend_layers {
            match payload_type {
                PayloadType::Cover => {
                    let Some(proof) = self.proofs_generator.get_next_core_proof().await else {
                        return Err(Error::NoMoreProofOfQuotas);
                    };
                    proofs.push(proof);
                }
                PayloadType::Data => {
                    let Some(proof) = self.proofs_generator.get_next_leadership_proof().await
                    else {
                        return Err(Error::NoMoreProofOfQuotas);
                    };
                    proofs.push(proof);
                }
            }
        }

        let membership_size = self.membership.size();
        let proofs_and_signing_keys = proofs
            .into_iter()
            .map(|proof| {
                let expected_index = proof
                    .proof_of_selection
                    .expected_index(membership_size)
                    .expect("Node index should exist.");
                (proof, expected_index)
            })
            .map(|(proof, node_index)| {
                (
                    proof,
                    self.membership
                        .get_node_at(node_index)
                        .expect("Node at index should exist.")
                        .public_key,
                )
            });

        let inputs = EncapsulationInputs::new(
            proofs_and_signing_keys
                .into_iter()
                .map(|(proof, receiver_non_ephemeral_signing_key)| {
                    EncapsulationInput::new(
                        proof.ephemeral_signing_key,
                        &receiver_non_ephemeral_signing_key,
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
