use groth16::{Fr, Groth16Input, Groth16InputDeser};
use serde::{Deserialize, Serialize};

use crate::{
    PolChainInputsData, PolWalletInputsData,
    chain_inputs::{PolChainInputs, PolChainInputsJson},
    wallet_inputs::{PolWalletInputs, PolWalletInputsJson},
};

/// The inputs to the circuit prover.
#[derive(Clone, Serialize)]
#[serde(into = "PolInputsJson", rename_all = "snake_case")]
pub struct PolWitnessInputs {
    pub wallet: PolWalletInputs,
    pub chain: PolChainInputs,
}

impl PolWitnessInputs {
    pub fn from_chain_and_wallet_data(
        chain: PolChainInputsData,
        wallet: PolWalletInputsData,
    ) -> Result<Self, <PolChainInputs as TryFrom<PolChainInputsData>>::Error> {
        Ok(Self {
            wallet: wallet.into(),
            chain: chain.try_into()?,
        })
    }
}

#[derive(Serialize)]
pub struct PolInputsJson {
    #[serde(flatten)]
    pub wallet: PolWalletInputsJson,
    #[serde(flatten)]
    pub chain: PolChainInputsJson,
}

impl From<&PolWitnessInputs> for PolInputsJson {
    fn from(inputs: &PolWitnessInputs) -> Self {
        Self {
            wallet: (&inputs.wallet).into(),
            chain: (&inputs.chain).into(),
        }
    }
}

impl From<PolWitnessInputs> for PolInputsJson {
    fn from(inputs: PolWitnessInputs) -> Self {
        Self {
            wallet: (&inputs.wallet).into(),
            chain: (&inputs.chain).into(),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct PolVerifierInputJson([Groth16InputDeser; 9]);

/// Public inputs of the POL verifier circuit as returned by the prover.
/// This inputs are the ones that need to be fed into the verifier.
pub struct PolVerifierInput {
    entropy_contribution: Groth16Input,
    slot_number: Groth16Input,
    epoch_nonce: Groth16Input,
    lottery_0: Groth16Input,
    lottery_1: Groth16Input,
    aged_root: Groth16Input,
    latest_root: Groth16Input,
    leader_pk1: Groth16Input,
    leader_pk2: Groth16Input,
}

impl TryFrom<PolVerifierInputJson> for PolVerifierInput {
    type Error = <Groth16Input as TryFrom<Groth16InputDeser>>::Error;

    fn try_from(value: PolVerifierInputJson) -> Result<Self, Self::Error> {
        let [
            entropy_contribution,
            slot_number,
            epoch_nonce,
            lottery_0,
            lottery_1,
            aged_root,
            latest_root,
            leader_pk1,
            leader_pk2,
        ] = value.0;
        Ok(Self {
            entropy_contribution: entropy_contribution.try_into()?,
            slot_number: slot_number.try_into()?,
            epoch_nonce: epoch_nonce.try_into()?,
            lottery_0: lottery_0.try_into()?,
            lottery_1: lottery_1.try_into()?,
            aged_root: aged_root.try_into()?,
            latest_root: latest_root.try_into()?,
            leader_pk1: leader_pk1.try_into()?,
            leader_pk2: leader_pk2.try_into()?,
        })
    }
}

impl PolVerifierInput {
    #[must_use]
    pub const fn to_inputs(&self) -> [Fr; 9] {
        [
            self.entropy_contribution.into_inner(),
            self.slot_number.into_inner(),
            self.epoch_nonce.into_inner(),
            self.lottery_0.into_inner(),
            self.lottery_1.into_inner(),
            self.aged_root.into_inner(),
            self.latest_root.into_inner(),
            self.leader_pk1.into_inner(),
            self.leader_pk2.into_inner(),
        ]
    }
}
