use std::{
    ops::{Div as _, Sub as _},
    sync::LazyLock,
};

use groth16::{Fr, Groth16Input, Groth16InputDeser};
use num_bigint::BigUint;
use primitive_types::U256;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    slot_number: Groth16InputDeser,
    epoch_nonce: Groth16InputDeser,
    lottery_0: Groth16InputDeser,
    lottery_1: Groth16InputDeser,
    aged_root: Groth16InputDeser,
    latest_root: Groth16InputDeser,
    #[serde(rename = "one_time_key_part_one")]
    leader_pk1: Groth16InputDeser,
    #[serde(rename = "one_time_key_part_two")]
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

static P: LazyLock<U256> = LazyLock::new(|| {
    U256::from_str_radix(
        "0x30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001",
        16,
    )
    .expect("P must be a valid hex string")
});
// t0 constant :
// 0x27b6fe27507ca57ca369280400c79b5d2f58ff94d87cb0fbfc8294eb69eb1ea
static T0_CONSTANT: LazyLock<U256> = LazyLock::new(|| {
    U256::from_str_radix(
        "0x27b6fe27507ca57ca369280400c79b5d2f58ff94d87cb0fbfc8294eb69eb1ea",
        16,
    )
    .expect("Constant must be a valid hex string")
});

// t1 constant:
// -0x104bfd09ebdd0a57772289d0973489b62662a4dc6f09da8b4af3c5cfb1dcdd
static T1_CONSTANT: LazyLock<U256> = LazyLock::new(|| {
    U256::from_str_radix(
        "0x104bfd09ebdd0a57772289d0973489b62662a4dc6f09da8b4af3c5cfb1dcdd",
        16,
    )
    .expect("Constant must be a valid hex string")
});

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
        let slot_number = U256::from(slot_number);
        if slot_number > *P {
            return Err(PolInputsFromDataError::SlotGreaterThanP);
        }
        let epoch_nonce = U256::from(epoch_nonce);
        if epoch_nonce > *P {
            return Err(PolInputsFromDataError::EpochGreaterThanP);
        }
        let total_stake = U256::from(total_stake);

        let lottery_0 = T0_CONSTANT.div(total_stake);
        let lottery_1 = P.sub(T1_CONSTANT.div(total_stake.pow(U256::from(2u8))));

        Ok(Self {
            slot_number: Groth16Input::new(Fr::from(BigUint::from_bytes_le(
                slot_number.to_little_endian().as_ref(),
            ))),
            epoch_nonce: Groth16Input::new(Fr::from(BigUint::from_bytes_le(
                epoch_nonce.to_little_endian().as_ref(),
            ))),
            lottery_0: Groth16Input::new(Fr::from(BigUint::from_bytes_le(
                lottery_0.to_little_endian().as_ref(),
            ))),
            lottery_1: Groth16Input::new(Fr::from(BigUint::from_bytes_le(
                lottery_1.to_little_endian().as_ref(),
            ))),
            aged_root: aged_root.into(),
            latest_root: latest_root.into(),
            leader_pk1: pk1.into(),
            leader_pk2: pk2.into(),
        })
    }
}
