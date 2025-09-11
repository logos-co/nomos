use groth16::{Field as _, Fr};
use num_bigint::BigUint;
use poq::{
    PoQBlendInputsData, PoQChainInputsData, PoQCommonInputsData, PoQInputsFromDataError,
    PoQWalletInputsData, PoQWitnessInputs,
};

use crate::crypto::proofs::quota::inputs::{
    prove::private::{ProofOfCoreQuotaInputs, ProofOfLeadershipQuotaInputs, ProofType},
    split_ephemeral_signing_key,
};

pub mod private;
pub mod public;
pub use self::{private::Inputs as PrivateInputs, public::Inputs as PublicInputs};

#[derive(Debug, Clone)]
pub(crate) struct Inputs {
    pub public: PublicInputs,
    pub private: PrivateInputs,
}

impl TryFrom<Inputs> for PoQWitnessInputs {
    type Error = PoQInputsFromDataError;

    fn try_from(value: Inputs) -> Result<Self, Self::Error> {
        let (signing_key_first_half, signing_key_second_half) =
            split_ephemeral_signing_key(value.public.signing_key);
        let (blend_input_data, wallet_input_data) =
            inputs_data_for_proof_type(value.private.proof_type);
        Ok(Self {
            chain: PoQChainInputsData {
                core_root: value.public.core_root,
                pol_epoch_nonce: value.public.pol_epoch_nonce,
                pol_ledger_aged: value.public.pol_ledger_aged,
                session: value.public.session,
                total_stake: value.public.total_stake,
            }
            .try_into()?,
            common: PoQCommonInputsData {
                core_quota: value.public.core_quota,
                index: value.private.key_index,
                leader_quota: value.public.leader_quota,
                message_key: (
                    BigUint::from_bytes_le(&signing_key_first_half[..]).into(),
                    BigUint::from_bytes_le(&signing_key_second_half[..]).into(),
                ),
                selector: value.private.selector,
            }
            .into(),
            blend: blend_input_data.into(),
            wallet: wallet_input_data.into(),
        })
    }
}

fn inputs_data_for_proof_type(proof_type: ProofType) -> (PoQBlendInputsData, PoQWalletInputsData) {
    match proof_type {
        ProofType::CoreQuota(ProofOfCoreQuotaInputs {
            core_path,
            core_path_selectors,
            core_sk,
        }) => (
            PoQBlendInputsData {
                core_path,
                core_path_selectors,
                core_sk,
            },
            PoQWalletInputsData {
                aged_path: vec![Fr::ZERO; 32],
                aged_selector: vec![false; 32],
                note_value: u64::MIN,
                output_number: u64::MIN,
                slot: u64::MIN,
                slot_secret: Fr::ZERO,
                slot_secret_path: vec![Fr::ZERO; 25],
                starting_slot: u64::MIN,
                transaction_hash: Fr::ZERO,
            },
        ),
        ProofType::LeadershipQuota(ProofOfLeadershipQuotaInputs {
            aged_path,
            aged_selector,
            note_value,
            output_number,
            slot,
            slot_secret,
            slot_secret_path,
            starting_slot,
            transaction_hash,
        }) => (
            PoQBlendInputsData {
                core_path: vec![Fr::ZERO; 20],
                core_path_selectors: vec![false; 20],
                core_sk: Fr::ZERO,
            },
            PoQWalletInputsData {
                aged_path,
                aged_selector,
                note_value,
                output_number,
                slot,
                slot_secret,
                slot_secret_path,
                starting_slot,
                transaction_hash,
            },
        ),
    }
}
