pub(super) mod selection_randomness {
    use groth16::{fr_from_bytes, fr_to_bytes};
    use nomos_core::crypto::ZkHash;
    use serde::{de::Error, Deserialize, Deserializer, Serialize as _, Serializer};

    pub fn serialize<S>(input: &ZkHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        fr_to_bytes(input).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ZkHash, D::Error>
    where
        D: Deserializer<'de>,
    {
        fr_from_bytes(&Deserialize::deserialize(deserializer)?).map_err(Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::codec::SerdeOp as _;

    use crate::crypto::proofs::selection::ProofOfSelection;

    #[test]
    fn serialize_deserialize() {
        let proof = ProofOfSelection::from_bytes_unchecked([0; _]);

        let serialized_proof = ProofOfSelection::serialize(&proof).unwrap();
        let deserialized_proof = ProofOfSelection::deserialize(&serialized_proof[..]).unwrap();

        assert!(proof == deserialized_proof);
    }
}
