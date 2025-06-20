use crypto_bigint::{CheckedMul as _, CheckedSub as _, Encoding as _, U256};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderPublic {
    pub epoch_nonce: [u8; 32],
    pub slot: u64,
    pub scaled_phi_approx: (U256, U256),
    pub entropy: [u8; 32],
    pub aged_root: [u8; 32],
    pub latest_root: [u8; 32],
    // TODO: missing rewards
}

impl LeaderPublic {
    #[must_use]
    pub fn new(
        aged_root: [u8; 32],
        latest_root: [u8; 32],
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
    pub fn check_winning(&self, value: u64, note_id: [u8; 32], sk: [u8; 16]) -> bool {
        let threshold = phi_approx(U256::from_u64(value), self.scaled_phi_approx);
        let ticket = ticket(note_id, sk, self.epoch_nonce, self.slot);
        ticket < threshold
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

fn ticket(note_id: [u8; 32], sk: [u8; 16], epoch_nonce: [u8; 32], slot: u64) -> U256 {
    let mut hasher = Sha256::new();
    hasher.update(b"LEAD_V1");
    hasher.update(epoch_nonce);
    hasher.update(slot.to_be_bytes());
    hasher.update(note_id);
    hasher.update(sk);

    let ticket_bytes: [u8; 32] = hasher.finalize().into();

    U256::from_be_bytes(ticket_bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderPrivate {
    // PLACEHOLDER: fix after mantle update
    pub value: u64,
    pub note_id: [u8; 32],
    pub sk: [u8; 16],
}
