use std::{
    ops::{Deref as _, Div as _},
    str::FromStr as _,
    sync::LazyLock,
};

use groth16::{Fr, Groth16Input, Groth16InputDeser};
use num_bigint::BigUint;
use num_traits::CheckedSub as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
static P: LazyLock<BigUint> = LazyLock::new(|| {
    BigUint::from_str(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
    )
    .expect("P constant should parse")
});

/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
static T0_CONSTANT: LazyLock<BigUint> = LazyLock::new(|| {
    // t0 constant :
    // 0x27b6fe27507ca57ca369280400c79b5d2f58ff94d87cb0fbfc8294eb69eb1ea
    BigUint::from_str(
        "1122720085251457488657939587576977282954863756865979276605118041105190793706",
    )
    .expect("Constant should parse")
});

/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
static T1_CONSTANT: LazyLock<BigUint> = LazyLock::new(|| {
    // t1 constant:
    // -0x104bfd09ebdd0a57772289d0973489b62662a4dc6f09da8b4af3c5cfb1dcdd
    BigUint::from_str("28794005923809446652337194229268641024881242442862297438215833784455126237")
        .expect("Constant should parse")
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
        let slot_number = BigUint::from(slot_number);
        if slot_number > *P {
            return Err(PolInputsFromDataError::SlotGreaterThanP);
        }
        let epoch_nonce = BigUint::from(epoch_nonce);
        if epoch_nonce > *P {
            return Err(PolInputsFromDataError::EpochGreaterThanP);
        }
        let total_stake = BigUint::from(total_stake);

        let lottery_0 = T0_CONSTANT.deref().div(total_stake.clone());
        let lottery_1 = P
            .checked_sub(&T1_CONSTANT.deref().div(total_stake.pow(2)))
            .expect("(T1 / (S^2)) must be less than P");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants() {
        let total_stake = 5000u32;
        assert_eq!(
            T0_CONSTANT.to_string(),
            "1122720085251457488657939587576977282954863756865979276605118041105190793706"
        );
        assert_eq!(
            T1_CONSTANT.to_string(),
            "28794005923809446652337194229268641024881242442862297438215833784455126237"
        );
        let lottery_0: BigUint = T0_CONSTANT.deref().div(total_stake);
        assert_eq!(
            lottery_0.to_string(),
            "224544017050291497731587917515395456590972751373195855321023608221038158"
        );
        let lottery_1 = P
            .checked_sub(&T1_CONSTANT.deref().div(total_stake.pow(2)))
            .expect("(T1 / (S^2)) must be less than P");
        assert_eq!(
            lottery_1.to_string(),
            "21888242870687514985294027879163787319377618759420784645983712289047175144239"
        );
    }
}
