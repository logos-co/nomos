mod hasher;
pub use ark_bn254::Fr;
use jf_poseidon2::Poseidon2;

pub type Poseidon2Bn254 = Poseidon2<Fr>;
pub type Poseidon2Bn254Hasher = hasher::Poseidon2Hasher;
pub type ZkHash = Fr;

pub trait Digest {
    fn digest(inputs: &[Fr]) -> ZkHash;

    fn new() -> Self;
    fn update(&mut self, input: &Fr);
    fn finalize(self) -> ZkHash;
}
