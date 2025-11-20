use poq::PoQProof;
use poseidon2::ZkHash;

pub const KEY_NULLIFIER_SIZE: usize = size_of::<ZkHash>();
pub const PROOF_CIRCUIT_SIZE: usize = size_of::<PoQProof>();
pub const PROOF_OF_QUOTA_SIZE: usize = KEY_NULLIFIER_SIZE.checked_add(PROOF_CIRCUIT_SIZE).unwrap();

pub const PROOF_OF_SELECTION_SIZE: usize = size_of::<ZkHash>();
