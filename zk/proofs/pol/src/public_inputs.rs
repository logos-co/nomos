use std::{
    ops::{Add, Div, Sub},
    sync::LazyLock,
};

use ark_bn254::Fq;
use ark_ec::pairing::Pairing;
use ark_ff::{BigInt, BigInteger};
use groth16::{Bn254, Fr, Groth16PublicInput};
use num_bigint::BigUint;
use primitive_types::U256;
use thiserror::Error;

pub struct PolPublicInputs {
    slot_number: Groth16PublicInput,
    epoch_nonce: Groth16PublicInput,
    lottery_0: Groth16PublicInput,
    lottery_1: Groth16PublicInput,
    aged_root: Groth16PublicInput,
    latest_root: Groth16PublicInput,
    leader_pk_1: Groth16PublicInput,
    leader_pk_2: Groth16PublicInput,
    entropy_contribution: Groth16PublicInput,
}

pub struct PolPublicInputsData {
    entropy_contribution: [u8; 32],
    slot_number: u64,
    epoch_nonce: u64,
    total_stake: u64,
    aged_root: [u8; 32],
    latest_root: [u8; 32],
    leader_pk: ([u8; 16], [u8; 16]),
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

impl TryFrom<PolPublicInputsData> for PolPublicInputs {
    type Error = PolInputsFromDataError;

    fn try_from(
        PolPublicInputsData {
            entropy_contribution,
            slot_number,
            epoch_nonce,
            total_stake,
            aged_root,
            latest_root,
            leader_pk: (pk1, pk2),
        }: PolPublicInputsData,
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
            slot_number: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                slot_number.to_little_endian().as_ref(),
            ))),
            epoch_nonce: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                epoch_nonce.to_little_endian().as_ref(),
            ))),
            lottery_0: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                lottery_0.to_little_endian().as_ref(),
            ))),
            lottery_1: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                lottery_1.to_little_endian().as_ref(),
            ))),
            aged_root: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                aged_root.as_ref(),
            ))),
            latest_root: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                latest_root.as_ref(),
            ))),
            leader_pk_1: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(pk1.as_ref()))),
            leader_pk_2: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(pk2.as_ref()))),
            entropy_contribution: Groth16PublicInput::new(Fr::from(BigUint::from_bytes_le(
                entropy_contribution.as_ref(),
            ))),
        })
    }
}

impl PolPublicInputs {
    pub const fn to_inputs(&self) -> [Fr; 9] {
        [
            self.entropy_contribution.into_inner(),
            self.slot_number.into_inner(),
            self.epoch_nonce.into_inner(),
            self.lottery_0.into_inner(),
            self.lottery_1.into_inner(),
            self.aged_root.into_inner(),
            self.latest_root.into_inner(),
            self.leader_pk_1.into_inner(),
            self.leader_pk_2.into_inner(),
        ]
    }
}
