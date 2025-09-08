use groth16::{Fr, Groth16Input, Groth16InputDeser};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::lottery::{P, compute_lottery_values};

/// Public inputs of the POL cirmcom circuit with circom specific types.
#[derive(Copy, Clone)]
pub struct PolChainInputs {
    slot_number: Groth16Input,
    epoch_nonce: Groth16Input,
    lottery_0: Groth16Input,
    lottery_1: Groth16Input,
    aged_root: Groth16Input,
    latest_root: Groth16Input,
    leader_pk1: Groth16Input,
    leader_pk2: Groth16Input,
}

/// Public inputs of the POL cirmcom circuit to be provided by the chain.
pub struct PolChainInputsData {
    pub slot_number: u64,
    pub epoch_nonce: u64,
    pub total_stake: u64,
    pub aged_root: Fr,
    pub latest_root: Fr,
    pub leader_pk: (Fr, Fr),
}

#[derive(Deserialize, Serialize)]
pub struct PolChainInputsJson {
    #[serde(rename = "sl")]
    slot_number: Groth16InputDeser,
    epoch_nonce: Groth16InputDeser,
    #[serde(rename = "t0")]
    lottery_0: Groth16InputDeser,
    #[serde(rename = "t1")]
    lottery_1: Groth16InputDeser,
    #[serde(rename = "ledger_aged")]
    aged_root: Groth16InputDeser,
    #[serde(rename = "ledger_latest")]
    latest_root: Groth16InputDeser,
    #[serde(rename = "P_lead_part_one")]
    leader_pk1: Groth16InputDeser,
    #[serde(rename = "P_lead_part_two")]
    leader_pk2: Groth16InputDeser,
}

impl TryFrom<PolChainInputsJson> for PolChainInputs {
    type Error = <Groth16Input as TryFrom<Groth16InputDeser>>::Error;

    fn try_from(
        PolChainInputsJson {
            slot_number,
            epoch_nonce,
            lottery_0,
            lottery_1,
            aged_root,
            latest_root,
            leader_pk1,
            leader_pk2,
        }: PolChainInputsJson,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
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

impl From<&PolChainInputs> for PolChainInputsJson {
    fn from(
        PolChainInputs {
            slot_number,
            epoch_nonce,
            lottery_0,
            lottery_1,
            aged_root,
            latest_root,
            leader_pk1: leader_pk_1,
            leader_pk2: leader_pk_2,
        }: &PolChainInputs,
    ) -> Self {
        Self {
            slot_number: slot_number.into(),
            epoch_nonce: epoch_nonce.into(),
            lottery_0: lottery_0.into(),
            lottery_1: lottery_1.into(),
            aged_root: aged_root.into(),
            latest_root: latest_root.into(),
            leader_pk1: leader_pk_1.into(),
            leader_pk2: leader_pk_2.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum PolInputsFromDataError {
    #[error("Slot number is greater than P")]
    SlotGreaterThanP,
    #[error("Epoch nonce is greater than P")]
    EpochGreaterThanP,
}

impl TryFrom<PolChainInputsData> for PolChainInputs {
    type Error = PolInputsFromDataError;

    fn try_from(
        PolChainInputsData {
            slot_number,
            epoch_nonce,
            total_stake,
            aged_root,
            latest_root,
            leader_pk: (pk1, pk2),
        }: PolChainInputsData,
    ) -> Result<Self, Self::Error> {
        let slot_number = BigUint::from(slot_number);
        if slot_number > *P {
            return Err(PolInputsFromDataError::SlotGreaterThanP);
        }
        let epoch_nonce = BigUint::from(epoch_nonce);
        if epoch_nonce > *P {
            return Err(PolInputsFromDataError::EpochGreaterThanP);
        }

        let (lottery_0, lottery_1) = compute_lottery_values(total_stake);

        Ok(Self {
            slot_number: Groth16Input::new(slot_number.into()),
            epoch_nonce: Groth16Input::new(epoch_nonce.into()),
            lottery_0: Groth16Input::new(lottery_0.into()),
            lottery_1: Groth16Input::new(lottery_1.into()),
            aged_root: aged_root.into(),
            latest_root: latest_root.into(),
            leader_pk1: pk1.into(),
            leader_pk2: pk2.into(),
        })
    }
}
