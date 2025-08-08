use ark_ec::{CurveGroup, pairing::Pairing};

#[derive(Eq, PartialEq)]
pub struct VerificationKey<E: Pairing> {
    pub alpha_1: E::G1,
    pub beta_2: E::G2,
    pub gamma_2: E::G2,
    pub delta_2: E::G2,
    pub ic: &'static [E::G1],
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
            alpha_g1: alpha_1.into_affine(),
            beta_g2: beta_2.into_affine(),
            gamma_g2: gamma_2.into_affine(),
            delta_g2: delta_2.into_affine(),
            gamma_abc_g1: ic.iter().map(|x| x.into_affine()).collect(),
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
