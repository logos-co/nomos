use blake2::Digest as _;
use groth16::{CompressedGroth16Proof, Groth16Input, Groth16PreparedVerificationKey, fr_to_bytes};

pub trait DevModeProof {
    fn vk(&self) -> &Groth16PreparedVerificationKey;
    fn public_inputs(&self) -> Vec<Groth16Input>;
}

/// Used to decide if we should produce real proofs or generate fake proofs for
/// development.
///
/// Returns `true` if the environment variable `ZK_DEV_MODE` is set to anything
/// other than "0" Otherwise `false` is returned
#[must_use]
pub fn zk_dev_mode_enabled() -> bool {
    std::env::var("ZK_DEV_MODE")
        .map(|v| v != "0")
        .unwrap_or(false)
}

pub fn prove_dev_mode(witness: &impl DevModeProof) -> CompressedGroth16Proof {
    eprintln!(
        "!!!! ZK_DEV_MODE is enabled. Proofs generated in this mode are unsafe and only to be used for development !!!!"
    );

    // Blake2b supports up to 512bit (64byte) hashes, meaning
    // only first half of the groth16 proof will be filled.
    let mut hasher = blake2::Blake2b512::new();
    hasher.update(witness.vk().to_bytes());
    for input in witness.public_inputs() {
        hasher.update(fr_to_bytes(&input.into_inner()));
    }

    let mut proof_bytes = [0u8; 128];
    proof_bytes[..64].copy_from_slice(&hasher.finalize());

    CompressedGroth16Proof::from_bytes(&proof_bytes)
}

pub fn verify_dev_mode(proof: &CompressedGroth16Proof, public_inputs: &impl DevModeProof) -> bool {
    &prove_dev_mode(public_inputs) == proof
}
