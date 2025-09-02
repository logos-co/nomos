use std::sync::LazyLock;

use groth16::{
    Groth16PreparedVerificationKey, Groth16VerificationKey, Groth16VerificationKeyJsonDeser,
};

pub struct PolVerifyingKey(Groth16PreparedVerificationKey);

impl AsRef<Groth16PreparedVerificationKey> for PolVerifyingKey {
    fn as_ref(&self) -> &Groth16PreparedVerificationKey {
        &self.0
    }
}

static POL_VK: LazyLock<PolVerifyingKey> = LazyLock::new(|| {
    let vk_json = include_bytes!("verification_key.json");
    let groth16_vk_json: Groth16VerificationKeyJsonDeser =
        serde_json::from_slice(vk_json).expect("Key should always be valid");
    let groth16_vk: Groth16VerificationKey = groth16_vk_json
        .try_into()
        .expect("Verification key should always be valid");
    PolVerifyingKey(groth16_vk.into_prepared())
});
