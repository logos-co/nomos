use core::hash::Hash;

use nomos_blend_message::{
    Error, PayloadType,
    crypto::proofs::quota::inputs::prove::private::ProofOfLeadershipQuotaInputs,
    input::EncapsulationInput,
};

use crate::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::{
        crypto::{EncapsulationInputs, SessionCryptographicProcessorSettings},
        provers::{ProofsGeneratorSettings, leader::LeaderProofsGenerator},
    },
    serialize_encapsulated_message,
};

/// [`SessionCryptographicProcessor`] is responsible for only wrapping data
/// messages (no cover messages) for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
///
/// This processor is suitable for non-core nodes that do not need to generate
/// any cover traffic and are hence only interested in blending data messages.
pub struct SessionCryptographicProcessor<NodeId, ProofsGenerator> {
    num_blend_layers: u64,
    membership: Membership<NodeId>,
    proofs_generator: ProofsGenerator,
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    ProofsGenerator: LeaderProofsGenerator,
{
    #[must_use]
    pub fn new(
        settings: &SessionCryptographicProcessorSettings,
        membership: Membership<NodeId>,
        public_info: ProofsGeneratorSettings,
        private_info: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        Self {
            num_blend_layers: settings.num_blend_layers,
            membership,
            proofs_generator: ProofsGenerator::new(public_info, private_info),
        }
    }
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    NodeId: Eq + Hash + 'static,
    ProofsGenerator: LeaderProofsGenerator,
{
    pub async fn encapsulate_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        let mut proofs = Vec::with_capacity(self.num_blend_layers as usize);

        for _ in 0..self.num_blend_layers {
            proofs.push(self.proofs_generator.get_next_proof().await);
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

        EncapsulatedMessage::new(&inputs, PayloadType::Data, payload)
    }

    pub async fn encapsulate_and_serialize_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_data_payload(payload).await?,
        ))
    }
}
