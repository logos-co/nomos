mod constants;
mod hasher;
mod params;

pub use ark_bn254::{Bn254, Fr};
use jf_poseidon2::sponge::Poseidon2SpongeState;
pub use jf_poseidon2::{Poseidon2, Poseidon2Error, Poseidon2Params};
pub use params::Poseidon2Bn254Params;
pub type Poseidon2Bn254 = Poseidon2<Fr>;
pub type Poseidon2SpongeBn254 = Poseidon2SpongeState<Fr, 3, 1, Poseidon2Bn254Params>;
