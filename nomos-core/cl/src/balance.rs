use rand_core::CryptoRngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::PartialTxWitness;

pub type Value = u64;
pub type Unit = [u8; 32];

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct Balance([u8; 32]);

impl Balance {
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct UnitBalance {
    pub unit: Unit,
    pub pos: u64,
    pub neg: u64,
}

impl UnitBalance {
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.pos == self.neg
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BalanceWitness {
    pub balances: Vec<UnitBalance>,
    pub blinding: [u8; 16],
}

impl BalanceWitness {
    pub fn random_blinding(mut rng: impl CryptoRngCore) -> [u8; 16] {
        let mut blinding = [0u8; 16];
        rng.fill_bytes(&mut blinding);

        blinding
    }

    #[must_use]
    pub fn zero(blinding: [u8; 16]) -> Self {
        Self {
            balances: Vec::default(),
            blinding,
        }
    }

    #[must_use]
    pub fn from_ptx(ptx: &PartialTxWitness, blinding: [u8; 16]) -> Self {
        let mut balance = Self::zero(blinding);

        for input in &ptx.inputs {
            balance.insert_negative(input.note.unit, input.note.value);
        }

        for output in &ptx.outputs {
            balance.insert_positive(output.note.unit, output.note.value);
        }

        balance.clear_zeros();

        balance
    }

    pub fn insert_positive(&mut self, unit: Unit, value: Value) {
        for unit_bal in &mut self.balances {
            if unit_bal.unit == unit {
                unit_bal.pos += value;
                return;
            }
        }

        // Unit was not found, so we must create one.
        self.balances.push(UnitBalance {
            unit,
            pos: value,
            neg: 0,
        });
    }

    pub fn insert_negative(&mut self, unit: Unit, value: Value) {
        for unit_bal in &mut self.balances {
            if unit_bal.unit == unit {
                unit_bal.neg += value;
                return;
            }
        }

        self.balances.push(UnitBalance {
            unit,
            pos: 0,
            neg: value,
        });
    }

    pub fn clear_zeros(&mut self) {
        let mut i = 0usize;
        while i < self.balances.len() {
            if self.balances[i].is_zero() {
                self.balances.swap_remove(i);
                // don't increment `i` since the last element has been swapped
                // into the `i`'th place
            } else {
                i += 1;
            }
        }
    }

    pub fn combine(balances: impl IntoIterator<Item = Self>, blinding: [u8; 16]) -> Self {
        let mut combined = Self::zero(blinding);

        for balance in balances {
            for unit_bal in &balance.balances {
                if unit_bal.pos > unit_bal.neg {
                    combined.insert_positive(unit_bal.unit, unit_bal.pos - unit_bal.neg);
                } else {
                    combined.insert_negative(unit_bal.unit, unit_bal.neg - unit_bal.pos);
                }
            }
        }

        combined.clear_zeros();

        combined
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.balances.is_empty()
    }

    #[must_use]
    pub fn commit(&self) -> Balance {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_CL_BAL_COMMIT");

        for unit_balance in &self.balances {
            hasher.update(unit_balance.unit);
            hasher.update(unit_balance.pos.to_le_bytes());
            hasher.update(unit_balance.neg.to_le_bytes());
        }
        hasher.update(self.blinding);

        let commit_bytes: [u8; 32] = hasher.finalize().into();
        Balance(commit_bytes)
    }
}
