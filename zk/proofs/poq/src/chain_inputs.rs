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

#[derive(Copy, Clone)]
pub struct PoQChainInputs {
    session: Groth16Input,
    core_root: Groth16Input,
    pol_ledger_aged: Groth16Input,
    pol_epoch_nonce: Groth16Input,
    pol_t0: Groth16Input,
    pol_t1: Groth16Input,
}

pub struct PoQChainInputsData {
    pub session: u64,
    pub core_root: Fr,
    pub pol_ledger_aged: Fr,
    pub pol_epoch_nonce: Fr,
    pub total_stake: u64,
}

#[derive(Deserialize, Serialize)]
pub struct PoQChainInputsJson {
    session: Groth16InputDeser,
    core_root: Groth16InputDeser,
    pol_ledger_aged: Groth16InputDeser,
    pol_epoch_nonce: Groth16InputDeser,
    pol_t0: Groth16InputDeser,
    pol_t1: Groth16InputDeser,
}

impl TryFrom<PoQChainInputsJson> for PoQChainInputs {
    type Error = <Groth16Input as TryFrom<Groth16InputDeser>>::Error;

    fn try_from(
        PoQChainInputsJson {
            session,
            core_root,
            pol_ledger_aged,
            pol_epoch_nonce,
            pol_t0,
            pol_t1,
        }: PoQChainInputsJson,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            session: session.try_into()?,
            core_root: core_root.try_into()?,
            pol_ledger_aged: pol_ledger_aged.try_into()?,
            pol_epoch_nonce: pol_epoch_nonce.try_into()?,
            pol_t0: pol_t0.try_into()?,
            pol_t1: pol_t1.try_into()?,
        })
    }
}

impl From<&PoQChainInputs> for PoQChainInputsJson {
    fn from(
        PoQChainInputs {
            session,
            core_root,
            pol_ledger_aged,
            pol_epoch_nonce,
            pol_t0,
            pol_t1,
        }: &PoQChainInputs,
    ) -> Self {
        Self {
            session: session.into(),
            core_root: core_root.into(),
            pol_ledger_aged: pol_ledger_aged.into(),
            pol_epoch_nonce: pol_epoch_nonce.into(),
            pol_t0: pol_t0.into(),
            pol_t1: pol_t1.into(),
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
pub enum PoQInputsFromDataError {
    #[error("Session number is greater than P")]
    SessionGreaterThanP,
}

impl TryFrom<PoQChainInputsData> for PoQChainInputs {
    type Error = PoQInputsFromDataError;

    fn try_from(
        PoQChainInputsData {
            session,
            core_root,
            pol_ledger_aged,
            pol_epoch_nonce,
            total_stake,
        }: PoQChainInputsData,
    ) -> Result<Self, Self::Error> {
        let session = BigUint::from(session);
        if session > *P {
            return Err(PoQInputsFromDataError::SessionGreaterThanP);
        }
        let total_stake = BigUint::from(total_stake);

        let lottery_0 = T0_CONSTANT.deref().div(total_stake.clone());
        let lottery_1 = P
            .checked_sub(&T1_CONSTANT.deref().div(total_stake.pow(2)))
            .expect("(T1 / (S^2)) must be less than P");

        Ok(Self {
            session: Groth16Input::new(session.into()),
            core_root: core_root.into(),
            pol_ledger_aged: pol_ledger_aged.into(),
            pol_epoch_nonce: pol_epoch_nonce.into(),
            pol_t0: Groth16Input::new(lottery_0.into()),
            pol_t1: Groth16Input::new(lottery_1.into()),
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
