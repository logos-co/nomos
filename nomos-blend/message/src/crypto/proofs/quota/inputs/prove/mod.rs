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
        let chain_input_data = PoQChainInputsData {
            core_root: value.public.core_root,
            pol_epoch_nonce: value.public.pol_epoch_nonce,
            pol_ledger_aged: value.public.pol_ledger_aged,
            session: value.public.session,
            total_stake: value.public.total_stake,
        };
        let common_input_data = PoQCommonInputsData {
            core_quota: value.public.core_quota,
            index: value.private.key_index,
            leader_quota: value.public.leader_quota,
            message_key: (
                BigUint::from_bytes_le(&signing_key_first_half[..]).into(),
                BigUint::from_bytes_le(&signing_key_second_half[..]).into(),
            ),
            selector: value.private.selector,
        };
        witness_input_for_proof_type(
            chain_input_data,
            common_input_data,
            value.private.proof_type,
        )
    }
}

fn witness_input_for_proof_type(
    chain_input_data: PoQChainInputsData,
    common_input_data: PoQCommonInputsData,
    proof_type: ProofType,
) -> Result<PoQWitnessInputs, PoQInputsFromDataError> {
    match proof_type {
        ProofType::CoreQuota(ProofOfCoreQuotaInputs {
            core_path,
            core_path_selectors,
            core_sk,
        }) => {
            let blend_input_data = PoQBlendInputsData {
                core_path,
                core_path_selectors,
                core_sk,
            };
            PoQWitnessInputs::from_core_node_data(
                chain_input_data,
                common_input_data,
                blend_input_data,
            )
        }
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
            ..
        }) => {
            let wallet_input_data = PoQWalletInputsData {
                aged_path,
                aged_selector,
                note_value,
                output_number,
                slot,
                slot_secret,
                slot_secret_path,
                starting_slot,
                transaction_hash,
            };
            PoQWitnessInputs::from_leader_data(
                chain_input_data,
                common_input_data,
                wallet_input_data,
            )
        }
    }
}
