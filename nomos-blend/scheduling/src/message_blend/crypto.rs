use std::hash::Hash;

use derivative::Derivative;
use nomos_blend_message::{
    crypto::{
        keys::{Ed25519PrivateKey, X25519PrivateKey},
        proofs::{
            quota::{
                PrivateInputs, ProofOfCoreQuotaPrivateInputs, ProofOfLeadershipQuotaPrivateInputs,
                ProofOfQuota, PublicInputs,
            },
            selection::{ProofOfSelection, ProofOfSelectionInput},
        },
    },
    encap::{
        DecapsulationOutput as InternalDecapsulationOutput,
        EncapsulatedMessage as InternalEncapsulatedMessage,
    },
    input::{EncapsulationInput, EncapsulationInputs as InternalEncapsulationInputs},
    Error, PayloadType,
};
use nomos_core::{crypto::ZkHash, wire};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{membership::Membership, serde::ed25519_privkey_hex};

const ENCAPSULATION_COUNT: usize = 3;
pub type EncapsulatedMessage = InternalEncapsulatedMessage<ENCAPSULATION_COUNT>;
pub type EncapsulationInputs = InternalEncapsulationInputs<ENCAPSULATION_COUNT>;
pub type UnwrappedMessage = InternalDecapsulationOutput<ENCAPSULATION_COUNT>;

struct GeneratedProofs {
    signing_key: Ed25519PrivateKey,
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
    key_nullifier: ZkHash,
}

struct ProofsStorage {
    proof_of_core_quotas: Vec<GeneratedProofs>,
    proof_of_leadership_quotas: Vec<GeneratedProofs>,
}

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
    fn new(
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
    ) -> Self {
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
                let (proof_of_quota, key_nullifier) = ProofOfQuota::new(
                    public_inputs,
                    PrivateInputs::new_proof_of_core_quota_inputs(key_index, private_inputs),
                );
                // TODO: Verifiably select the recipient of each message layer based on the spec: https://www.notion.so/nomos-tech/Message-Encapsulation-Mechanism-215261aa09df81309d7fd7f1c2da086b?source=copy_link#215261aa09df81319317cb1e86b2e527.
                // TODO: Generate a proof of selection and link it to the secret selection randomness: https://www.notion.so/nomos-tech/Blend-Protocol-215261aa09df81ae8857d71066a80084?source=copy_link#215261aa09df8110a059d0eb477559b5.
                GeneratedProofs {
                    signing_key,
                    proof_of_quota,
                    key_nullifier,
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
                let (proof_of_quota, key_nullifier) = ProofOfQuota::new(
                    public_inputs,
                    PrivateInputs::new_proof_of_leadership_quota_inputs(key_index, private_inputs),
                );
                GeneratedProofs {
                    signing_key,
                    proof_of_quota,
                    key_nullifier,
                }
            })
            .collect();

        Self {
            proof_of_core_quotas,
            proof_of_leadership_quotas,
        }
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

impl<NodeId, Rng> CryptographicProcessor<NodeId, Rng> {
    pub fn new(
        settings: CryptographicProcessorSettings,
        membership: Membership<NodeId>,
        session_info: SessionInfo,
        rng: Rng,
    ) -> Self {
        // Derive the non-ephemeral encryption key
        // from the non-ephemeral signing key.
        Self {
            encryption_private_key: settings.non_ephemeral_signing_key.derive_x25519(),
            membership,
            non_ephemeral_quota_key: settings.non_ephemeral_quota_key,
            non_ephemeral_signing_key: settings.non_ephemeral_signing_key,
            num_blend_layers: settings.num_blend_layers as usize,
            proof_of_quota_storage: ProofsStorage::new(
                session_info,
                settings.non_ephemeral_quota_key,
            ),
            rng,
        }
    }

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
    NodeId: Eq + Hash,
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
        // Retrieve the non-ephemeral signing keys of the blend nodes
        // TODO: Change logic used to select nodes based on the spec: https://www.notion.so/nomos-tech/Message-Encapsulation-Mechanism-215261aa09df81309d7fd7f1c2da086b?source=copy_link#215261aa09df81319317cb1e86b2e527.
        let blend_node_signing_keys = self
            .membership
            .choose_remote_nodes(&mut self.rng, self.num_blend_layers)
            .map(|node| node.public_key)
            .collect::<Vec<_>>();

        let inputs = EncapsulationInputs::new(
            blend_node_signing_keys
                .iter()
                .map(|blend_node_signing_key| {
                    // Retrieve a pre-computed ephemeral signing key, proof of quota, and proof of
                    // selection for each encapsulation.
                    let ephemeral_signing_key = Ed25519PrivateKey::generate();
                    EncapsulationInput::new(
                        ephemeral_signing_key,
                        blend_node_signing_key,
                        ProofOfQuota::dummy(),
                        ProofOfSelection::dummy(),
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
    wire::serialize(&message).expect("EncapsulatedMessage should be serializable")
}

pub fn deserialize_encapsulated_message(message: &[u8]) -> Result<EncapsulatedMessage, Error> {
    wire::deserialize(message).map_err(|_| Error::DeserializationFailed)
}
