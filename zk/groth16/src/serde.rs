pub mod serde_fr {
    use ark_bn254::Fr;
    use ark_ff::{PrimeField as _, biginteger::BigInteger as _};
    use num_bigint::BigUint;
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S>(item: &Fr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex = hex::encode(item.into_bigint().to_bytes_le()); // Convert `Fr` to hex representation
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Fr, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let bytes = hex::decode(hex_str).map_err(serde::de::Error::custom)?;
        Ok(BigUint::from_bytes_le(&bytes).into()) // Parse from hex
    }

    #[cfg(test)]
    mod tests {
        use ark_bn254::Fr;
        use num_bigint::BigUint;
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, Eq, PartialEq, Debug)]
        struct FrDeser(#[serde(with = "crate::serde::serde_fr")] pub Fr);
        #[test]
        fn test_serialize_deserialize() {
            let fr1 = FrDeser(BigUint::from(123u8).into());
            let v = serde_json::to_string(&fr1).unwrap();
            let fr2: FrDeser = serde_json::from_str(&v).unwrap();
            assert_eq!(fr1, fr2);
        }
    }
}
