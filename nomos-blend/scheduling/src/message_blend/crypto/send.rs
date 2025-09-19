use core::hash::Hash;

use nomos_blend_message::{
    crypto::keys::X25519PrivateKey, input::EncapsulationInput, Error, PayloadType,
};

use crate::{
    membership::Membership,
    message_blend::{
        crypto::EncapsulationInputs, ProofsGenerator as ProofsGeneratorTrait,
        SessionCryptographicProcessorSettings, SessionInfo,
    },
    serialize_encapsulated_message, EncapsulatedMessage,
};

/// [`SessionCryptographicProcessor`] is responsible for only wrapping
/// messages for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
pub struct SessionCryptographicProcessor<NodeId, ProofsGenerator> {
    num_blend_layers: u64,
    /// The non-ephemeral encryption key (NEK) for decapsulating messages.
    non_ephemeral_encryption_key: X25519PrivateKey,
    membership: Membership<NodeId>,
    proofs_generator: ProofsGenerator,
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator> {
    pub(super) const fn non_ephemeral_encryption_key(&self) -> &X25519PrivateKey {
        &self.non_ephemeral_encryption_key
    }

    pub(super) const fn membership(&self) -> &Membership<NodeId> {
        &self.membership
    }
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    ProofsGenerator: ProofsGeneratorTrait,
{
    #[must_use]
    pub fn new(
        settings: &SessionCryptographicProcessorSettings,
        membership: Membership<NodeId>,
        session_info: SessionInfo,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let non_ephemeral_encryption_key = settings.non_ephemeral_signing_key.derive_x25519();
        Self {
            num_blend_layers: settings.num_blend_layers,
            non_ephemeral_encryption_key,
            membership,
            proofs_generator: ProofsGenerator::new(session_info),
        }
    }
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
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
        let mut proofs = Vec::with_capacity(self.num_blend_layers as usize);

        match payload_type {
            PayloadType::Cover => {
                for _ in 0..self.num_blend_layers {
                    let Some(proof) = self.proofs_generator.get_next_core_proof().await else {
                        return Err(Error::NoMoreProofOfQuotas);
                    };
                    proofs.push(proof);
                }
            }
            PayloadType::Data => {
                for _ in 0..self.num_blend_layers {
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
