use nomos_core::crypto::ZkHash;
use num_bigint::BigUint;
use poq::{PoQVerifierInput, PoQVerifierInputData};

use crate::crypto::proofs::quota::inputs::{prove::PublicInputs, split_ephemeral_signing_key};

/// Set of inputs required to verify a Proof of Quota.
///
/// It includes the inputs used to generate the proof (which must be fetched
/// from the verifier's context), and the proof key nullifier, which is part of
/// the Proof of Quota that is included in a Blend header.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub prove_inputs: PublicInputs,
    pub key_nullifier: ZkHash,
}

impl Inputs {
    #[must_use]
    pub const fn from_prove_inputs_and_nullifier(
        prove_inputs: PublicInputs,
        key_nullifier: ZkHash,
    ) -> Self {
        Self {
            prove_inputs,
            key_nullifier,
        }
    }
}

impl From<Inputs> for PoQVerifierInput {
    fn from(value: Inputs) -> Self {
        let (signing_key_first_half, signing_key_second_half) =
            split_ephemeral_signing_key(value.prove_inputs.signing_key);
        PoQVerifierInputData {
            core_quota: value.prove_inputs.core_quota,
            core_root: value.prove_inputs.core_root,
            k_part_one: BigUint::from_bytes_le(&signing_key_first_half[..]).into(),
            k_part_two: BigUint::from_bytes_le(&signing_key_second_half[..]).into(),
            key_nullifier: value.key_nullifier,
            leader_quota: value.prove_inputs.leader_quota,
            pol_epoch_nonce: value.prove_inputs.pol_epoch_nonce,
            pol_ledger_aged: value.prove_inputs.pol_ledger_aged,
            session: value.prove_inputs.session,
            total_stake: value.prove_inputs.total_stake,
        }
        .into()
    }
}
