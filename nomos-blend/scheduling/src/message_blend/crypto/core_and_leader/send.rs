use core::hash::Hash;

use nomos_blend_message::{
    Error, PayloadType,
    crypto::{
        keys::X25519PrivateKey,
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::inputs::prove::{
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
                public::LeaderInputs,
            },
        },
    },
    input::EncapsulationInput,
};

use crate::{
    EncapsulatedMessage,
    membership::Membership,
    message_blend::{
        crypto::{EncapsulationInputs, SessionCryptographicProcessorSettings},
        provers::{ProofsGeneratorSettings, core_and_leader::CoreAndLeaderProofsGenerator},
    },
    serialize_encapsulated_message,
};

/// [`SessionCryptographicProcessor`] is responsible for only wrapping
/// cover and data messages for the message indistinguishability.
///
/// Each instance is meant to be used during a single session.
///
/// This processor is suitable for non-core nodes that want to generate noise
/// (i.e., cover) traffic along with data payloads.
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

    #[cfg(test)]
    pub const fn proofs_generator(&self) -> &ProofsGenerator {
        &self.proofs_generator
    }
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    ProofsGenerator: CoreAndLeaderProofsGenerator,
{
    #[must_use]
    pub fn new(
        settings: &SessionCryptographicProcessorSettings,
        membership: Membership<NodeId>,
        public_info: PoQVerificationInputsMinusSigningKey,
        private_core_info: ProofOfCoreQuotaInputs,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let non_ephemeral_encryption_key = settings.non_ephemeral_signing_key.derive_x25519();
        let generator_settings = ProofsGeneratorSettings {
            local_node_index: membership.local_index(),
            membership_size: membership.size(),
            public_inputs: public_info,
        };
        Self {
            num_blend_layers: settings.num_blend_layers,
            non_ephemeral_encryption_key,
            membership,
            proofs_generator: ProofsGenerator::new(generator_settings, private_core_info),
        }
    }

    pub fn rotate_epoch(&mut self, new_epoch_public_info: LeaderInputs) {
        self.proofs_generator.rotate_epoch(new_epoch_public_info);
    }

    pub fn set_epoch_private(&mut self, new_epoch_private: ProofOfLeadershipQuotaInputs) {
        self.proofs_generator.set_epoch_private(new_epoch_private);
    }
}

impl<NodeId, ProofsGenerator> SessionCryptographicProcessor<NodeId, ProofsGenerator>
where
    NodeId: Eq + Hash + 'static,
    ProofsGenerator: CoreAndLeaderProofsGenerator,
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
                    let Some(proof) = self.proofs_generator.get_next_leader_proof().await else {
                        return Err(Error::NoLeadershipInfoProvided);
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

#[cfg(test)]
mod test {
    use groth16::Field as _;
    use multiaddr::{Multiaddr, PeerId};
    use nomos_blend_message::crypto::{
        keys::Ed25519PrivateKey,
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::inputs::prove::{
                private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs},
                public::{CoreInputs, LeaderInputs},
            },
        },
    };
    use nomos_core::crypto::ZkHash;

    use super::SessionCryptographicProcessor;
    use crate::{
        membership::{Membership, Node},
        message_blend::crypto::{
            SessionCryptographicProcessorSettings,
            test_utils::TestEpochChangeCoreAndLeaderProofsGenerator,
        },
    };

    #[test]
    fn epoch_rotation() {
        let mut processor =
            SessionCryptographicProcessor::<_, TestEpochChangeCoreAndLeaderProofsGenerator>::new(
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
                ProofOfCoreQuotaInputs {
                    core_path: vec![],
                    core_path_selectors: vec![],
                    core_sk: ZkHash::ZERO,
                },
            );

        let new_leader_inputs = LeaderInputs {
            pol_ledger_aged: ZkHash::ONE,
            pol_epoch_nonce: ZkHash::ONE,
            message_quota: 2,
            total_stake: 2,
        };

        processor.rotate_epoch(new_leader_inputs);

        assert_eq!(
            processor.proofs_generator.0.public_inputs.leader,
            new_leader_inputs
        );
    }

    #[test]
    fn set_epoch_private() {
        let mut processor =
            SessionCryptographicProcessor::<_, TestEpochChangeCoreAndLeaderProofsGenerator>::new(
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
                ProofOfCoreQuotaInputs {
                    core_path: vec![],
                    core_path_selectors: vec![],
                    core_sk: ZkHash::ZERO,
                },
            );

        let new_private_inputs = ProofOfLeadershipQuotaInputs {
            aged_path: vec![ZkHash::ONE],
            aged_selector: vec![true],
            note_value: 2,
            output_number: 2,
            pol_secret_key: ZkHash::ONE,
            slot: 2,
            slot_secret: ZkHash::ONE,
            slot_secret_path: vec![ZkHash::ONE],
            starting_slot: 2,
            transaction_hash: ZkHash::ONE,
        };

        processor.set_epoch_private(new_private_inputs.clone());

        assert_eq!(processor.proofs_generator.1, Some(new_private_inputs));
    }
}
