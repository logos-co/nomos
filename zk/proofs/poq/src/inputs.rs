use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::{Deserialize, Serialize};

use crate::{
    PoQChainInputsData, PoQCommonInputsData,
    blend_inputs::{PoQBlendInputs, PoQBlendInputsData, PoQBlendInputsJson},
    chain_inputs::{PoQChainInputs, PoQChainInputsJson},
    common_inputs::{PoQCommonInputs, PoQCommonInputsJson},
    wallet_inputs::{PoQWalletInputs, PoQWalletInputsData, PoQWalletInputsJson},
};

#[derive(Clone, Serialize)]
#[serde(into = "PoQInputsJson", rename_all = "snake_case")]
pub struct PoQWitnessInputs {
    pub chain: PoQChainInputs,
    pub common: PoQCommonInputs,
    pub blend: PoQBlendInputs,
    pub wallet: PoQWalletInputs,
}

impl PoQWitnessInputs {
    pub fn from_leader_data(
        chain: PoQChainInputsData,
        common: PoQCommonInputsData,
        wallet: PoQWalletInputsData,
    ) -> Result<Self, <PoQChainInputs as TryFrom<PoQChainInputsData>>::Error> {
        Ok(Self {
            chain: chain.try_into()?,
            common: common.into(),
            blend: PoQBlendInputs::from(PoQBlendInputsData {
                core_sk: Fr::ZERO,
                core_path: vec![Fr::ZERO; 20],
                core_path_selectors: vec![false; 20],
            }),
            wallet: wallet.into(),
        })
    }

    pub fn from_core_node_data(
        chain: PoQChainInputs,
        common: PoQCommonInputs,
        blend: PoQBlendInputs,
    ) -> Result<Self, <PoQChainInputs as TryFrom<PoQChainInputsData>>::Error> {
        Ok(Self {
            chain,
            common,
            blend,
            wallet: PoQWalletInputs::from(PoQWalletInputsData {
                slot: 2,
                note_value: 0,
                transaction_hash: Fr::ZERO,
                output_number: 0,
                aged_path: vec![Fr::ZERO; 32],
                aged_selector: vec![false; 32],
                slot_secret: Fr::ZERO,
                slot_secret_path: vec![Fr::ZERO; 25],
                starting_slot: 1,
            }),
        })
    }
}

#[derive(Serialize)]
pub struct PoQInputsJson {
    #[serde(flatten)]
    pub chain: PoQChainInputsJson,
    #[serde(flatten)]
    pub common: PoQCommonInputsJson,
    #[serde(flatten)]
    pub blend: PoQBlendInputsJson,
    #[serde(flatten)]
    pub wallet: PoQWalletInputsJson,
}

impl From<&PoQWitnessInputs> for PoQInputsJson {
    fn from(inputs: &PoQWitnessInputs) -> Self {
        Self {
            wallet: (&inputs.wallet).into(),
            chain: (&inputs.chain).into(),
            common: (&inputs.common).into(),
            blend: (&inputs.blend).into(),
        }
    }
}

impl From<PoQWitnessInputs> for PoQInputsJson {
    fn from(inputs: PoQWitnessInputs) -> Self {
        Self {
            wallet: (&inputs.wallet).into(),
            chain: (&inputs.chain).into(),
            common: (&inputs.common).into(),
            blend: (&inputs.blend).into(),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct PoQVerifierInputJson([Groth16InputDeser; 11]);

pub struct PoQVerifierInput {
    pub key_nullifier: Groth16Input,
    pub session: Groth16Input,
    pub core_quota: Groth16Input,
    pub leader_quota: Groth16Input,
    pub core_root: Groth16Input,
    pub k_part_one: Groth16Input,
    pub k_part_two: Groth16Input,
    pub pol_epoch_nonce: Groth16Input,
    pub pol_t0: Groth16Input,
    pub pol_t1: Groth16Input,
    pub pol_ledger_aged: Groth16Input,
}

impl TryFrom<PoQVerifierInputJson> for PoQVerifierInput {
    type Error = <Groth16Input as TryFrom<Groth16InputDeser>>::Error;

    fn try_from(value: PoQVerifierInputJson) -> Result<Self, Self::Error> {
        let [
            key_nullifier,
            session,
            core_quota,
            leader_quota,
            core_root,
            k_part_one,
            k_part_two,
            pol_epoch_nonce,
            pol_t0,
            pol_t1,
            pol_ledger_aged,
        ] = value.0;
        Ok(Self {
            key_nullifier: key_nullifier.try_into()?,
            session: session.try_into()?,
            core_quota: core_quota.try_into()?,
            leader_quota: leader_quota.try_into()?,
            core_root: core_root.try_into()?,
            k_part_one: k_part_one.try_into()?,
            k_part_two: k_part_two.try_into()?,
            pol_epoch_nonce: pol_epoch_nonce.try_into()?,
            pol_t0: pol_t0.try_into()?,
            pol_t1: pol_t1.try_into()?,
            pol_ledger_aged: pol_ledger_aged.try_into()?,
        })
    }
}

impl PoQVerifierInput {
    #[must_use]
    pub const fn to_inputs(&self) -> [Fr; 11] {
        [
            self.key_nullifier.into_inner(),
            self.session.into_inner(),
            self.core_quota.into_inner(),
            self.leader_quota.into_inner(),
            self.core_root.into_inner(),
            self.k_part_one.into_inner(),
            self.k_part_two.into_inner(),
            self.pol_epoch_nonce.into_inner(),
            self.pol_t0.into_inner(),
            self.pol_t1.into_inner(),
            self.pol_ledger_aged.into_inner(),
        ]
    }
}
