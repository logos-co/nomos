use core::ops::{Deref, DerefMut};
use std::hash::Hash;

use derivative::Derivative;
use nomos_blend_message::{
    crypto::keys::{Ed25519PrivateKey, X25519PrivateKey},
    encap::{
        decapsulated::DecapsulationOutput as InternalDecapsulationOutput,
        encapsulated::EncapsulatedMessage as InternalEncapsulatedMessage,
        validated::{
            IncomingEncapsulatedMessageWithValidatedPublicHeader as InternalIncomingEncapsulatedMessageWithValidatedPublicHeader,
            OutgoingEncapsulatedMessageWithValidatedPublicHeader as InternalOutgoingEncapsulatedMessageWithValidatedPublicHeader,
            RequiredProofOfSelectionVerificationInputs,
        },
        ProofsVerifier as ProofsVerifierTrait,
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
pub type IncomingEncapsulatedMessageWithValidatedPublicHeader =
    InternalIncomingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>;
pub type OutgoingEncapsulatedMessageWithValidatedPublicHeader =
    InternalOutgoingEncapsulatedMessageWithValidatedPublicHeader<ENCAPSULATION_COUNT>;

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

pub struct SenderSessionCryptographicProcessor<NodeId, ProofsGenerator> {
    settings: CryptographicProcessorSettings,
    /// The non-ephemeral encryption key (NEK) for decapsulating messages.
    non_ephemeral_encryption_key: X25519PrivateKey,
    membership: Membership<NodeId>,
    proofs_generator: ProofsGenerator,
}

impl<NodeId, ProofsGenerator> SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    ProofsGenerator: ProofsGeneratorTrait,
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
        }
    }
}

impl<NodeId, ProofsGenerator> SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>
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

        match payload_type {
            PayloadType::Cover => {
                for _ in 0..self.settings.num_blend_layers {
                    let Some(proof) = self.proofs_generator.get_next_core_proof().await else {
                        return Err(Error::NoMoreProofOfQuotas);
                    };
                    proofs.push(proof);
                }
            }
            PayloadType::Data => {
                for _ in 0..self.settings.num_blend_layers {
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
            // Collect remote (or local) index info for each PoSel.
            .map(|proof| {
                let expected_index = proof
                    .proof_of_selection
                    .expected_index(membership_size)
                    .expect("Node index should exist.");
                (proof, expected_index)
            })
            // Map retrieved indices to the nodes' public keys.
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

/// [`SessionCryptographicProcessor`] is responsible for wrapping and unwrapping
/// messages for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
pub struct SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier> {
    sender_processor: SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>,
    proofs_verifier: ProofsVerifier,
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsGenerator: ProofsGeneratorTrait,
    ProofsVerifier: ProofsVerifierTrait,
{
    #[must_use]
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        session_info: SessionInfo,
    ) -> Self {
        SenderSessionCryptographicProcessor::new(settings, membership, session_info).into()
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    From<SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>>
    for SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsVerifier: ProofsVerifierTrait,
{
    fn from(value: SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>) -> Self {
        Self {
            sender_processor: value,
            proofs_verifier: ProofsVerifier::new(),
        }
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier>
    SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
where
    ProofsVerifier: ProofsVerifierTrait,
{
    pub fn decapsulate_message(
        &self,
        message: IncomingEncapsulatedMessageWithValidatedPublicHeader,
    ) -> Result<DecapsulationOutput, Error> {
        let Some(local_node_index) = self.sender_processor.membership.local_index() else {
            return Err(Error::NotCoreNodeReceiver);
        };
        message.decapsulate(
            &self.sender_processor.non_ephemeral_encryption_key,
            &RequiredProofOfSelectionVerificationInputs {
                expected_node_index: local_node_index as u64,
                total_membership_size: self.sender_processor.membership.size() as u64,
            },
            &self.proofs_verifier,
        )
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier> Deref
    for SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    type Target = SenderSessionCryptographicProcessor<NodeId, ProofsGenerator>;

    fn deref(&self) -> &Self::Target {
        &self.sender_processor
    }
}

impl<NodeId, ProofsGenerator, ProofsVerifier> DerefMut
    for SenderAndReceiverCryptographicProcessor<NodeId, ProofsGenerator, ProofsVerifier>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sender_processor
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
