pub(super) mod proof {
    use generic_array::GenericArray;
    use groth16::CompressSize;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(remote = "groth16::CompressedProof")]
    pub struct SerializablePoQProof<E: CompressSize> {
        pub pi_a: GenericArray<u8, E::G1CompressedSize>,
        pub pi_b: GenericArray<u8, E::G2CompressedSize>,
        pub pi_c: GenericArray<u8, E::G1CompressedSize>,
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::codec::{DeserializeOp as _, SerializeOp as _};

    use crate::crypto::proofs::quota::ProofOfQuota;

    #[test]
    fn serialize_deserialize() {
        let proof = ProofOfQuota::from_bytes_unchecked([0; _]);

        let serialized_proof = &proof.to_bytes().unwrap();
        let deserialized_proof = ProofOfQuota::from_bytes(&serialized_proof[..]).unwrap();

        assert!(proof == deserialized_proof);
    }
}
