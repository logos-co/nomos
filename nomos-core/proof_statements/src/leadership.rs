use crypto_bigint::{CheckedMul as _, CheckedSub as _, Encoding as _, U256};
use groth16::{serde::serde_fr, Fr};
use num_bigint::BigUint;
use poseidon2::{Digest as _, Poseidon2Bn254Hasher};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderPublic {
    pub epoch_nonce: [u8; 32],
    pub slot: u64,
    pub scaled_phi_approx: (U256, U256),
    pub entropy: [u8; 32],
    #[serde(with = "serde_fr")]
    pub aged_root: Fr,
    #[serde(with = "serde_fr")]
    pub latest_root: Fr,
    // TODO: missing rewards
}

impl LeaderPublic {
    #[must_use]
    pub fn new(
        aged_root: Fr,
        latest_root: Fr,
        entropy: [u8; 32],
        epoch_nonce: [u8; 32],
        slot: u64,
        active_slot_coefficient: f64,
        total_stake: u64,
    ) -> Self {
        let total_stake_big = U256::from_u64(total_stake);
        let total_stake_sq_big = total_stake_big.checked_mul(&total_stake_big).unwrap();
        let double_total_stake_sq_big = total_stake_sq_big.checked_mul(&U256::from_u64(2)).unwrap();

        let precision_u64 = u64::MAX;
        let precision_big = U256::from_u64(u64::MAX);
        let precision_f64 = precision_u64 as f64;
        let order: U256 = U256::MAX;

        let order_div_precision = order.checked_div(&precision_big).unwrap();
        let order_div_precision_sq = order_div_precision.checked_div(&precision_big).unwrap();
        let neg_f_ln: U256 =
            U256::from_u64(((-f64::ln(1f64 - active_slot_coefficient)) * precision_f64) as u64);
        let neg_f_ln_sq = neg_f_ln.checked_mul(&neg_f_ln).unwrap();

        let neg_f_ln_order: U256 = order_div_precision.checked_mul(&neg_f_ln).unwrap();
        let t0 = neg_f_ln_order.checked_div(&total_stake_big).unwrap();
        let t1 = order_div_precision_sq
            .checked_mul(&neg_f_ln_sq)
            .unwrap()
            .checked_div(&double_total_stake_sq_big)
            .unwrap();

        Self {
            aged_root,
            latest_root,
            epoch_nonce,
            slot,
            entropy,
            scaled_phi_approx: (t0, t1),
        }
    }

    #[must_use]
    pub fn check_winning(&self, value: u64, note_id: Fr, sk: Fr) -> bool {
        let threshold = phi_approx(U256::from_u64(value), self.scaled_phi_approx);
        let threshold = BigUint::from_bytes_le(&threshold.to_le_bytes());
        let ticket = ticket(
            note_id,
            sk,
            BigUint::from_bytes_le(&self.epoch_nonce).into(),
            BigUint::from(self.slot).into(),
        );
        ticket < threshold.into()
    }
}

fn phi_approx(stake: U256, approx: (U256, U256)) -> U256 {
    // stake * (t0 - t1 * stake)
    stake
        .checked_mul(
            &approx
                .0
                .checked_sub(&approx.1.checked_mul(&stake).unwrap())
                .unwrap(),
        )
        .unwrap()
}

fn ticket(note_id: Fr, sk: Fr, epoch_nonce: Fr, slot: Fr) -> Fr {
    Poseidon2Bn254Hasher::digest(&[note_id, sk, epoch_nonce, slot])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderPrivate {
    // PLACEHOLDER: fix after mantle update
    pub value: u64,
    #[serde(with = "serde_fr")]
    pub note_id: Fr,
    #[serde(with = "serde_fr")]
    pub sk: Fr,
}
