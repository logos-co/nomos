use ark_ec::pairing::Pairing;

#[derive(Eq, PartialEq)]
pub struct VerificationKey<E: Pairing> {
    pub alpha_1: E::G1Affine,
    pub beta_2: E::G2Affine,
    pub gamma_2: E::G2Affine,
    pub delta_2: E::G2Affine,
    pub ic: Vec<E::G1Affine>,
}
pub struct PreparedVerificationKey<E: Pairing> {
    vk: ark_groth16::PreparedVerifyingKey<E>,
}

impl<E: Pairing> From<VerificationKey<E>> for ark_groth16::VerifyingKey<E> {
    fn from(value: VerificationKey<E>) -> Self {
        let VerificationKey {
            alpha_1,
            beta_2,
            gamma_2,
            delta_2,
            ic,
        } = value;

        Self {
            alpha_g1: alpha_1,
            beta_g2: beta_2,
            gamma_g2: gamma_2,
            delta_g2: delta_2,
            gamma_abc_g1: ic,
        }
    }
}

impl<E: Pairing> VerificationKey<E> {
    pub fn into_prepared(self) -> PreparedVerificationKey<E> {
        let vk: ark_groth16::VerifyingKey<E> = self.into();
        PreparedVerificationKey { vk: vk.into() }
    }
}

impl<E: Pairing> AsRef<ark_groth16::PreparedVerifyingKey<E>> for PreparedVerificationKey<E> {
    fn as_ref(&self) -> &ark_groth16::PreparedVerifyingKey<E> {
        &self.vk
    }
}
