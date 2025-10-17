use core::hash::Hash;

use nomos_blend_message::{
    Error, PayloadType,
    crypto::proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::inputs::prove::{private::ProofOfLeadershipQuotaInputs, public::LeaderInputs},
    },
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
        public_info: PoQVerificationInputsMinusSigningKey,
        private_info: ProofOfLeadershipQuotaInputs,
    ) -> Self {
        let generator_settings = ProofsGeneratorSettings {
            local_node_index: membership.local_index(),
            membership_size: membership.size(),
            public_inputs: public_info,
        };
        Self {
            num_blend_layers: settings.num_blend_layers,
            membership,
            proofs_generator: ProofsGenerator::new(generator_settings, private_info),
        }
    }

    pub fn rotate_epoch(
        &mut self,
        new_epoch_public: LeaderInputs,
        new_private_inputs: ProofOfLeadershipQuotaInputs,
    ) {
        self.proofs_generator
            .rotate_epoch(new_epoch_public, new_private_inputs);
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

#[cfg(test)]
mod test {
    use groth16::Field as _;
    use libp2p::{Multiaddr, PeerId};
    use nomos_blend_message::crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::inputs::prove::{
                private::ProofOfLeadershipQuotaInputs,
                public::{CoreInputs, LeaderInputs},
            },
        },
    };
    use nomos_core::crypto::ZkHash;

    use super::SessionCryptographicProcessor;
    use crate::{
        membership::{Membership, Node},
        message_blend::crypto::{
            SessionCryptographicProcessorSettings, test_utils::TestEpochChangeLeaderProofsGenerator,
        },
    };

    #[test]
    fn epoch_rotation() {
        let mut processor =
            SessionCryptographicProcessor::<_, TestEpochChangeLeaderProofsGenerator>::new(
                &SessionCryptographicProcessorSettings {
                    non_ephemeral_signing_key: Ed25519PrivateKey::generate(),
                    num_blend_layers: 1,
                },
                Membership::new_without_local(&[Node {
                    address: Multiaddr::empty(),
                    id: PeerId::random(),
                    public_key: [0; _].try_into().unwrap(),
                }]),
                PoQVerificationInputsMinusSigningKey {
                    session: 1,
                    core: CoreInputs {
                        quota: 1,
                        zk_root: ZkHash::ZERO,
                    },
                    leader: LeaderInputs {
                        message_quota: 1,
                        pol_epoch_nonce: ZkHash::ZERO,
                        pol_ledger_aged: ZkHash::ZERO,
                        total_stake: 1,
                    },
                },
                ProofOfLeadershipQuotaInputs {
                    aged_path_and_selectors: [(ZkHash::ZERO, false); _],
                    note_value: 1,
                    output_number: 1,
                    pol_secret_key: ZkHash::ZERO,
                    slot: 1,
                    slot_secret: ZkHash::ZERO,
                    slot_secret_path: [ZkHash::ZERO; _],
                    starting_slot: 1,
                    transaction_hash: ZkHash::ZERO,
                },
            );

        let new_leader_inputs = LeaderInputs {
            pol_ledger_aged: ZkHash::ONE,
            pol_epoch_nonce: ZkHash::ONE,
            message_quota: 2,
            total_stake: 2,
        };
        let new_private_inputs = ProofOfLeadershipQuotaInputs {
            aged_path_and_selectors: [(ZkHash::ONE, true); _],
            note_value: 2,
            output_number: 2,
            pol_secret_key: ZkHash::ONE,
            slot: 2,
            slot_secret: ZkHash::ONE,
            slot_secret_path: [ZkHash::ONE; _],
            starting_slot: 2,
            transaction_hash: ZkHash::ONE,
        };

        processor.rotate_epoch(new_leader_inputs, new_private_inputs.clone());

        assert_eq!(
            processor.proofs_generator.0.public_inputs.leader,
            new_leader_inputs
        );
        assert_eq!(processor.proofs_generator.1, new_private_inputs);
    }
}
