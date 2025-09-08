use ark_ff::{BigInteger as _, PrimeField as _};
use groth16::Fr;
use num_bigint::BigUint;
use serde::{Deserializer, Serializer};

use crate::block::References;

#[derive(serde::Serialize, serde::Deserialize)]
struct WireData {
    service_reward: Option<Vec<u8>>,
    mempool_transactions: Vec<Vec<u8>>,
}

impl serde::Serialize for References {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let service_reward = self.service_reward.as_ref().map(|fr| {
            let big_int = fr.into_bigint();
            big_int.to_bytes_le()
        });

        let mempool_transactions = self
            .mempool_transactions
            .iter()
            .map(|fr| {
                let big_int = fr.into_bigint();
                big_int.to_bytes_le()
            })
            .collect();

        let wire = WireData {
            service_reward,
            mempool_transactions,
        };

        wire.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for References {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire: WireData = WireData::deserialize(deserializer)?;

        let service_reward = wire.service_reward.map(|bytes| {
            let big_int = BigUint::from_bytes_le(&bytes);
            Fr::from(big_int)
        });

        let mempool_transactions = wire
            .mempool_transactions
            .into_iter()
            .map(|bytes| {
                let big_int = BigUint::from_bytes_le(&bytes);
                Fr::from(big_int)
            })
            .collect();

        Ok(Self {
            service_reward,
            mempool_transactions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Fr;
    use crate::block::References;

    #[test]
    fn test_references_wire_serialization() {
        let service_reward = Some(Fr::from(42u64));
        let mempool_transactions = vec![Fr::from(123u64), Fr::from(456u64), Fr::from(789u64)];

        let references = References {
            service_reward,
            mempool_transactions,
        };

        let wire_bytes =
            crate::wire::serialize(&references).expect("Failed to serialize with wire");
        let deserialized: References =
            crate::wire::deserialize(&wire_bytes).expect("Failed to deserialize with wire");

        assert_eq!(references.service_reward, deserialized.service_reward);
        assert_eq!(
            references.mempool_transactions,
            deserialized.mempool_transactions
        );

        assert_eq!(deserialized.service_reward.unwrap(), Fr::from(42u64));
        assert_eq!(deserialized.mempool_transactions.len(), 3);
        assert_eq!(deserialized.mempool_transactions[0], Fr::from(123u64));
        assert_eq!(deserialized.mempool_transactions[1], Fr::from(456u64));
        assert_eq!(deserialized.mempool_transactions[2], Fr::from(789u64));
    }
}
