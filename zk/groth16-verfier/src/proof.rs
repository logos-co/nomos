use ark_ec::{CurveGroup, pairing::Pairing};

use crate::protocol::Protocol;

pub struct Proof<E: Pairing> {
    pi_a: E::G1,
    pi_b: E::G2,
    pi_c: E::G1,
}

impl<E: Pairing> From<&Proof<E>> for ark_groth16::Proof<E> {
    fn from(value: &Proof<E>) -> Self {
        let Proof { pi_a, pi_b, pi_c } = value;
        Self {
            a: pi_a.into_affine(),
            b: pi_b.into_affine(),
            c: pi_c.into_affine(),
        }
    }
}
