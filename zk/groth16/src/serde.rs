pub mod serde_fr {
    use ark_bn254::Fr;
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    use crate::{FrBytes, fr_from_bytes_le, fr_to_bytes_le};

    pub fn serialize<S>(item: &Fr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes = fr_to_bytes_le(item);
        if serializer.is_human_readable() {
            let hex = hex::encode(bytes); // Convert `Fr` to hex representation
            serializer.serialize_str(&hex)
        } else {
            bytes.serialize(serializer)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Fr, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex_str = String::deserialize(deserializer)?;
            let bytes: FrBytes = hex::decode(hex_str)
                .map_err(serde::de::Error::custom)?
                .try_into()
                .map_err(|b: Vec<u8>| {
                    serde::de::Error::custom(format!("Bytes length is not 32, got: {:?}", b.len()))
                })?;
            Ok(fr_from_bytes_le(&bytes).map_err(serde::de::Error::custom)?) // Parse from hex
        } else {
            let sized_bytes = <[u8; 32]>::deserialize(deserializer)?;
            Ok(fr_from_bytes_le(&sized_bytes).map_err(serde::de::Error::custom)?)
        }
    }

    #[cfg(test)]
    mod tests {
        use ark_bn254::Fr;
        use num_bigint::BigUint;
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, Eq, PartialEq, Debug)]
        struct FrDeser(#[serde(with = "crate::serde::serde_fr")] pub Fr);
        #[test]
        fn test_serialize_deserialize_json() {
            let fr1 = FrDeser(BigUint::from(123u8).into());
            let v = serde_json::to_string(&fr1).unwrap();
            let fr2: FrDeser = serde_json::from_str(&v).unwrap();
            assert_eq!(fr1, fr2);
        }

        #[test]
        fn test_serialize_deserialize_bin() {
            let fr1 = FrDeser(BigUint::from(123u8).into());
            let v = bincode::serialize(&fr1).unwrap();
            let fr2: FrDeser = bincode::deserialize(&v).unwrap();
            assert_eq!(fr1, fr2);
        }
    }
}
