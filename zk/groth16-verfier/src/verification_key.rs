use ark_bn254::Bn254;
use ark_ec::{CurveGroup, pairing::Pairing};
use ark_groth16::VerifyingKey;

#[derive(Eq, PartialEq)]
pub struct VerificationKey<E: Pairing> {
    pub public_size: usize,
    pub alpha_1: E::G1,
    pub beta_2: E::G2,
    pub gamma_2: E::G2,
    pub delta_2: E::G2,
    pub alpha_beta_12: E::TargetField,
    pub ic: &'static [E::G1],
}

impl<E: Pairing> VerificationKey<E> {
    #[must_use]
    pub fn public_size(&self) -> usize {
        self.public_size
    }
}

impl<E: Pairing> From<VerificationKey<E>> for VerifyingKey<E> {
    fn from(value: VerificationKey<E>) -> Self {
        let VerificationKey {
            public_size,
            alpha_1,
            beta_2,
            gamma_2,
            delta_2,
            alpha_beta_12,
            ic,
        } = value;

        Self {
            alpha_g1: alpha_1.into_affine(),
            beta_g2: beta_2.into_affine(),
            gamma_g2: gamma_2.into_affine(),
            delta_g2: delta_2.into_affine(),
            gamma_abc_g1: ic.into_iter().map(|x| x.into_affine()).collect(),
        }
    }
}
