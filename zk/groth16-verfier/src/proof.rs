use ark_ec::pairing::Pairing;

use crate::protocol::Protocol;

pub struct Proof<E: Pairing> {
    protocol: Protocol,
    pi_a: E::G1,
    pi_b: E::G2,
    pi_c: E::G1,
}
