mod serde;

use ::serde::{Deserialize, Serialize};
use generic_array::{ArrayLength, GenericArray};
use groth16::{Bn254, CompressSize, fr_from_bytes, fr_from_bytes_unchecked, fr_to_bytes};
use poq::PoQProof;
use poseidon2::ZkHash;

pub const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
pub const PROOF_CIRCUIT_SIZE: usize = size_of::<PoQProof>();
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProofOfQuota {
    #[serde(with = "groth16::serde::serde_fr")]
    pub key_nullifier: ZkHash,
    #[serde(with = "self::serde::proof::SerializablePoQProof")]
    pub proof: PoQProof,
}

impl ProofOfQuota {
    #[must_use]
    pub fn from_bytes_unchecked(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Self {
        let (key_nullifier_bytes, proof_circuit_bytes) = bytes.split_at(KEY_NULLIFIER_SIZE);
        let key_nullifier = fr_from_bytes_unchecked(key_nullifier_bytes);
        let (pi_a, pi_b, pi_c) = split_proof_components::<
            <Bn254 as CompressSize>::G1CompressedSize,
            <Bn254 as CompressSize>::G2CompressedSize,
        >(proof_circuit_bytes.try_into().unwrap());

        Self {
            key_nullifier,
            proof: PoQProof::new(pi_a, pi_b, pi_c),
        }
    }
}

impl TryFrom<[u8; PROOF_OF_QUOTA_SIZE]> for ProofOfQuota {
    type Error = Box<dyn std::error::Error>;

    fn try_from(bytes: [u8; PROOF_OF_QUOTA_SIZE]) -> Result<Self, Self::Error> {
        let (key_nullifier_bytes, proof_circuit_bytes) = bytes.split_at(KEY_NULLIFIER_SIZE);
        let key_nullifier = fr_from_bytes(key_nullifier_bytes).map_err(Box::new)?;
        let (pi_a, pi_b, pi_c) = split_proof_components::<
            <Bn254 as CompressSize>::G1CompressedSize,
            <Bn254 as CompressSize>::G2CompressedSize,
        >(proof_circuit_bytes.try_into().map_err(Box::new)?);

        Ok(Self {
            key_nullifier,
            proof: PoQProof::new(pi_a, pi_b, pi_c),
        })
    }
}

impl From<&ProofOfQuota> for [u8; PROOF_OF_QUOTA_SIZE] {
    fn from(proof: &ProofOfQuota) -> Self {
        let mut bytes = [0u8; PROOF_OF_QUOTA_SIZE];
        bytes[..KEY_NULLIFIER_SIZE].copy_from_slice(&fr_to_bytes(&proof.key_nullifier));
        bytes[KEY_NULLIFIER_SIZE..].copy_from_slice(&proof.proof.to_bytes());
        bytes
    }
}

fn split_proof_components<G1Compressed, G2Compressed>(
    bytes: [u8; PROOF_CIRCUIT_SIZE],
) -> (
    GenericArray<u8, G1Compressed>,
    GenericArray<u8, G2Compressed>,
    GenericArray<u8, G1Compressed>,
)
where
    G1Compressed: ArrayLength,
    G2Compressed: ArrayLength,
{
    let first_point_end_index = G1Compressed::USIZE;
    let second_point_end_index = first_point_end_index
        .checked_add(G2Compressed::USIZE)
        .expect("Second index overflow");
    let third_point_end_index = second_point_end_index
        .checked_add(G1Compressed::USIZE)
        .expect("Third index overflow");

    (
        GenericArray::try_from_iter(
            bytes
                .get(..first_point_end_index)
                .expect("Input byte array is not large enough for the first G1 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
        GenericArray::try_from_iter(
            bytes
                .get(first_point_end_index..second_point_end_index)
                .expect("Input byte array is not large enough for the first G2 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
        GenericArray::try_from_iter(
            bytes
                .get(second_point_end_index..third_point_end_index)
                .expect("Input byte array is not large enough for the second G1 compressed point.")
                .iter()
                .copied(),
        )
        .unwrap(),
    )
}
