use std::{collections::VecDeque, hash::Hash};

use derivative::Derivative;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, Ed25519PublicKey, X25519PrivateKey},
        proofs::{
            quota::{
                PrivateInputs, ProofOfCoreQuotaPrivateInputs, ProofOfLeadershipQuotaPrivateInputs,
                ProofOfQuota, PublicInputs,
            },
            selection::{ProofOfSelection, ProofOfSelectionInputs},
        },
    },
    encap::{
        DecapsulationOutput as InternalDecapsulationOutput,
        EncapsulatedMessage as InternalEncapsulatedMessage,
    },
    input::{EncapsulationInput, EncapsulationInputs as InternalEncapsulationInputs},
    Error, PayloadType,
};
use nomos_core::{
    crypto::{ZkHash, ZkHasher},
    wire,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{membership::Membership, serde::ed25519_privkey_hex};

const ENCAPSULATION_COUNT: usize = 3;
pub type EncapsulatedMessage = InternalEncapsulatedMessage<ENCAPSULATION_COUNT>;
pub type EncapsulationInputs = InternalEncapsulationInputs<ENCAPSULATION_COUNT>;
pub type UnwrappedMessage = InternalDecapsulationOutput<ENCAPSULATION_COUNT>;

pub struct GeneratedProofs {
    signing_key: Ed25519PrivateKey,
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
    recipient_non_ephemeral_verifying_key: Ed25519PublicKey,
}

struct ProofsStorage {
    proof_of_core_quotas: VecDeque<GeneratedProofs>,
    proof_of_leadership_quotas: VecDeque<GeneratedProofs>,
}

// TODO: Remove this `Default` impl when all the pieces are implemented
#[derive(Default)]
pub struct SessionInfo {
    number: u64,
    core_quota: usize,
    leader_quota: usize,
    core_root: ZkHash,
    pol_epoch_nonce: u64,
    pol_t0: u64,
    pol_t1: u64,
    pol_ledger_aged: ZkHash,
}

impl ProofsStorage {
    fn new<NodeId>(
        SessionInfo {
            core_quota,
            core_root,
            leader_quota,
            number,
            pol_epoch_nonce,
            pol_ledger_aged,
            pol_t0,
            pol_t1,
        }: SessionInfo,
        secret_key: ZkHash,
        membership: &Membership<NodeId>,
    ) -> Self
    where
        NodeId: Eq + Hash,
    {
        let proof_of_core_quotas = (0..=core_quota)
            .map(|key_index| {
                let signing_key = Ed25519PrivateKey::generate();
                let public_inputs = PublicInputs {
                    core_quota,
                    core_root,
                    leader_quota,
                    pol_epoch_nonce,
                    pol_ledger_aged,
                    pol_t0,
                    pol_t1,
                    session_number: number,
                    signing_key: signing_key.public_key(),
                };
                // TODO: Retrieve actual values
                let private_inputs = ProofOfCoreQuotaPrivateInputs {
                    core_sk: secret_key,
                    ..Default::default()
                };
                let proof_of_quota = ProofOfQuota::new(
                    public_inputs,
                    PrivateInputs::new_proof_of_core_quota_inputs(key_index, private_inputs),
                );
                let proof_of_selection = ProofOfSelection::new(ProofOfSelectionInputs {
                    ephemeral_key_index: key_index,
                    secret_key,
                    session_number: number,
                });
                // TODO: Calculate recipient node index.
                let recipient_node_index = {
                    let mut hasher = ZkHasher::new();
                    hasher.update(&[*proof_of_selection.as_ref()]);
                    // let res = hasher.finalize();
                    // let index = (res.0 as usize) % membership.size();
                    0usize
                };
                let recipient_non_ephemeral_verifying_key = membership
                    .get_remote_node_at(recipient_node_index)
                    .unwrap()
                    .public_key;
                GeneratedProofs {
                    signing_key,
                    proof_of_quota,
                    proof_of_selection,
                    recipient_non_ephemeral_verifying_key,
                }
            })
            .collect();

        let proof_of_leadership_quotas = (0..=leader_quota)
            .map(|key_index| {
                let signing_key = Ed25519PrivateKey::generate();
                let public_inputs = PublicInputs {
                    core_quota,
                    core_root,
                    leader_quota,
                    pol_epoch_nonce,
                    pol_ledger_aged,
                    pol_t0,
                    pol_t1,
                    session_number: number,
                    signing_key: signing_key.public_key(),
                };
                // TODO: Retrieve actual values
                let private_inputs = ProofOfLeadershipQuotaPrivateInputs::default();
                let proof_of_quota = ProofOfQuota::new(
                    public_inputs,
                    PrivateInputs::new_proof_of_leadership_quota_inputs(key_index, private_inputs),
                );
                let proof_of_selection = ProofOfSelection::new(ProofOfSelectionInputs {
                    ephemeral_key_index: key_index,
                    secret_key,
                    session_number: number,
                });
                // TODO: Calculate recipient node index.
                let recipient_node_index = {
                    let mut hasher = ZkHasher::new();
                    hasher.update(&[*proof_of_selection.as_ref()]);
                    // let res = hasher.finalize();
                    // let index = (res.0 as usize) % membership.size();
                    0usize
                };
                let recipient_non_ephemeral_verifying_key = membership
                    .get_remote_node_at(recipient_node_index)
                    .unwrap()
                    .public_key;
                GeneratedProofs {
                    signing_key,
                    proof_of_quota,
                    proof_of_selection,
                    recipient_non_ephemeral_verifying_key,
                }
            })
            .collect();

        Self {
            proof_of_core_quotas,
            proof_of_leadership_quotas,
        }
    }

    fn pop_next_proof_of_core_quota(&mut self) -> Option<GeneratedProofs> {
        self.proof_of_core_quotas.pop_front()
    }

    fn pop_next_proof_of_leadership_quota(&mut self) -> Option<GeneratedProofs> {
        self.proof_of_leadership_quotas.pop_front()
    }
}

/// [`CryptographicProcessor`] is responsible for wrapping and unwrapping
/// messages for the message indistinguishability.
pub struct CryptographicProcessor<NodeId, Rng> {
    /// The non-ephemeral encryption key for decapsulating messages.
    encryption_private_key: X25519PrivateKey,
    membership: Membership<NodeId>,
    rng: Rng,
    proof_of_quota_storage: ProofsStorage,
    non_ephemeral_signing_key: Ed25519PrivateKey,
    num_blend_layers: usize,
    non_ephemeral_quota_key: ZkHash,
}

#[derive(Clone, Derivative, Serialize, Deserialize)]
#[derivative(Debug)]
pub struct CryptographicProcessorSettings {
    /// The non-ephemeral signing key corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: Ed25519PrivateKey,
    /// `ß_c`: expected number of blending operations for each locally generated
    /// message.
    pub num_blend_layers: u64,
    #[serde(with = "groth16::serde::serde_fr")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_quota_key: ZkHash,
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng>
where
    NodeId: Eq + Hash,
{
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        rng: Rng,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        let proof_of_quota_storage = ProofsStorage::new(
            // TODO: Replace with input session_info.
            SessionInfo::default(),
            settings.non_ephemeral_quota_key,
            &membership,
        );
        Self {
            encryption_private_key: settings.non_ephemeral_signing_key.derive_x25519(),
            membership,
            non_ephemeral_quota_key: settings.non_ephemeral_quota_key,
            non_ephemeral_signing_key: settings.non_ephemeral_signing_key,
            num_blend_layers: settings.num_blend_layers as usize,
            proof_of_quota_storage,
            rng,
        }
    }
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng> {
    pub fn decapsulate_serialized_message(
        &self,
        message: &[u8],
    ) -> Result<UnwrappedMessage, Error> {
        self.decapsulate_message(deserialize_encapsulated_message(message)?)
    }

    pub fn decapsulate_message(
        &self,
        message: EncapsulatedMessage,
    ) -> Result<UnwrappedMessage, Error> {
        message.decapsulate(&self.encryption_private_key)
    }
}

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng>
where
    NodeId: Eq + Hash + Clone,
    Rng: RngCore,
{
    pub fn encapsulate_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Cover, payload)
    }

    pub fn encapsulate_and_serialize_cover_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_cover_payload(payload)?,
        ))
    }

    pub fn encapsulate_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        self.encapsulate_payload(PayloadType::Data, payload)
    }

    pub fn encapsulate_and_serialize_data_payload(
        &mut self,
        payload: &[u8],
    ) -> Result<Vec<u8>, Error> {
        Ok(serialize_encapsulated_message(
            &self.encapsulate_data_payload(payload)?,
        ))
    }

    fn encapsulate_payload(
        &mut self,
        payload_type: PayloadType,
        payload: &[u8],
    ) -> Result<EncapsulatedMessage, Error> {
        let (Some(first_layer_proof), Some(second_layer_proof), Some(third_layer_proof)) =
            (match payload_type {
                PayloadType::Cover => (
                    self.proof_of_quota_storage.pop_next_proof_of_core_quota(),
                    self.proof_of_quota_storage.pop_next_proof_of_core_quota(),
                    self.proof_of_quota_storage.pop_next_proof_of_core_quota(),
                ),
                PayloadType::Data => (
                    self.proof_of_quota_storage
                        .pop_next_proof_of_leadership_quota(),
                    self.proof_of_quota_storage
                        .pop_next_proof_of_leadership_quota(),
                    self.proof_of_quota_storage
                        .pop_next_proof_of_leadership_quota(),
                ),
            })
        else {
            // TODO: Ideally, we want to still use whatever proof is left, e.g., if only 1
            // or 2 proofs are left, we want to encapsulate the message only once or twice.
            return Err(Error::NoProofOfQuotasLeft);
        };

        let encapsulation_inputs = EncapsulationInputs::new(
            [first_layer_proof, second_layer_proof, third_layer_proof]
                .into_iter()
                .map(
                    |GeneratedProofs {
                         proof_of_quota,
                         proof_of_selection,
                         recipient_non_ephemeral_verifying_key,
                         signing_key,
                     }| {
                        EncapsulationInput::new(
                            signing_key,
                            &recipient_non_ephemeral_verifying_key,
                            proof_of_quota,
                            proof_of_selection,
                        )
                    },
                )
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )?;

        EncapsulatedMessage::new(&encapsulation_inputs, payload_type, payload)
    }
}

#[must_use]
pub fn serialize_encapsulated_message(message: &EncapsulatedMessage) -> Vec<u8> {
    wire::serialize(&message).expect("EncapsulatedMessage should be serializable")
}

pub fn deserialize_encapsulated_message(message: &[u8]) -> Result<EncapsulatedMessage, Error> {
    wire::deserialize(message).map_err(|_| Error::DeserializationFailed)
}
