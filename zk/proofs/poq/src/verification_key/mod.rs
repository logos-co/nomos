use std::sync::LazyLock;

use groth16::{
    Groth16PreparedVerificationKey, Groth16VerificationKey, Groth16VerificationKeyJsonDeser,
};

const CIRCUIT_NAME: &str = "poq";

pub struct PoQVerifyingKey(Groth16PreparedVerificationKey);

impl AsRef<Groth16PreparedVerificationKey> for PoQVerifyingKey {
    fn as_ref(&self) -> &Groth16PreparedVerificationKey {
        &self.0
    }
}

pub static POQ_VK: LazyLock<PoQVerifyingKey> = LazyLock::new(|| {
    let vk_path = circuits_utils::verification_key_path(CIRCUIT_NAME);
    let vk_json = std::fs::read(&vk_path).unwrap_or_else(|error| {
        panic!(
            "Failed to read verification key from {}: {}",
            vk_path.display(),
            error
        )
    });
    let groth16_vk_json: Groth16VerificationKeyJsonDeser =
        serde_json::from_slice(&vk_json).expect("Key should always be valid");
    let groth16_vk: Groth16VerificationKey = groth16_vk_json
        .try_into()
        .expect("Verification key should always be valid");
    PoQVerifyingKey(groth16_vk.into_prepared())
});
