mod hasher;
use ark_bn254::Fr;
use jf_poseidon2::Poseidon2;

pub type Poseidon2Bn254 = Poseidon2<Fr>;
pub type Poseidon2Bn254Hasher = hasher::Poseidon2Hasher;
