use std::num::NonZeroUsize;

use num_bigint::BigUint;
use zksign::{PublicKey, SecretKey};

/// Collection of wallet accounts that should be funded at genesis.
#[derive(Clone, Default, Debug)]
pub struct WalletConfig {
    pub accounts: Vec<WalletAccount>,
}

impl WalletConfig {
    #[must_use]
    pub const fn new(accounts: Vec<WalletAccount>) -> Self {
        Self { accounts }
    }

    #[must_use]
    pub fn uniform(total_funds: u64, users: NonZeroUsize) -> Self {
        let user_count = users.get() as u64;
        assert!(user_count > 0, "wallet user count must be non-zero");
        assert!(
            total_funds >= user_count,
            "wallet funds must allocate at least 1 token per user"
        );

        let base_allocation = total_funds / user_count;
        let mut remainder = total_funds % user_count;

        let accounts = (0..users.get())
            .map(|idx| {
                let mut amount = base_allocation;
                if remainder > 0 {
                    amount += 1;
                    remainder -= 1;
                }

                WalletAccount::deterministic(idx as u64, amount)
            })
            .collect();

        Self { accounts }
    }
}

/// Wallet account that holds funds in the genesis state.
#[derive(Clone, Debug)]
pub struct WalletAccount {
    pub label: String,
    pub secret_key: SecretKey,
    pub value: u64,
}

impl WalletAccount {
    #[must_use]
    pub fn new(label: impl Into<String>, secret_key: SecretKey, value: u64) -> Self {
        assert!(value > 0, "wallet account value must be positive");
        Self {
            label: label.into(),
            secret_key,
            value,
        }
    }

    #[must_use]
    pub fn deterministic(index: u64, value: u64) -> Self {
        let mut seed = [0u8; 32];
        seed[..2].copy_from_slice(b"wl");
        seed[2..10].copy_from_slice(&index.to_le_bytes());

        let secret_key = SecretKey::from(BigUint::from_bytes_le(&seed));
        Self::new(format!("wallet-user-{index}"), secret_key, value)
    }

    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        self.secret_key.to_public_key()
    }
}
