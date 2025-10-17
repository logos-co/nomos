use ark_bn254::Fr;
use ark_ff::AdditiveGroup;
use ark_ff::Field as _;

use crate::{Digest, Poseidon2Bn254};

#[derive(Debug)]
pub struct Poseidon2Hasher {
    state: [Fr; 3],
}

impl Clone for Poseidon2Hasher {
    fn clone(&self) -> Self {
        Self {
            state: self.state.to_vec().try_into().unwrap(),
        }
    }
}

impl Default for Poseidon2Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Poseidon2Hasher {
    #[must_use]
    pub const fn new() -> Self {
        let state = [Fr::ZERO, Fr::ZERO, Fr::ZERO];
        Self { state }
    }

    fn update_one(&mut self, input: &Fr) {
        self.state[0] += input;
        Poseidon2Bn254::permute_mut::<jf_poseidon2::constants::bn254::Poseidon2ParamsBn3, 3>(
            &mut self.state,
        );
    }

    pub fn update(&mut self, input: &[Fr]) {
        for fr in input {
            self.update_one(fr);
        }
        self.update_one(&Fr::ONE);
    }

    pub const fn finalize(self) -> Fr {
        self.state[0]
    }
}

impl Digest for Poseidon2Hasher {
    fn digest(inputs: &[Fr]) -> Fr {
        let mut hasher = Self::new();
        hasher.update(inputs);
        hasher.finalize()
    }

    fn new() -> Self {
        Self::new()
    }

    fn update(&mut self, input: &Fr) {
        Self::update_one(self, input);
    }

    fn finalize(mut self) -> Fr {
        Self::update_one(&mut self, &Fr::ONE);
        Self::finalize(self)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;

    use super::*;

    fn test_hasher(input: &[Fr], expected: Fr) {
        let mut hasher = Poseidon2Hasher::new();
        hasher.update(input);
        let result = hasher.finalize();
        assert_eq!(result, expected);
    }
    #[test]
    fn test_hashes() {
        // 0
        let expected_zero = Fr::from_str(
            "14440562208246903332530876912784724937356723424375796042690034647976142142243",
        )
        .unwrap();
        test_hasher(&[Fr::from(0)], expected_zero);
        // 1
        let expected_one = Fr::from_str(
            "13955187255749411516377601857453481686854514827536340092448578824571923228920",
        )
        .unwrap();
        test_hasher(&[Fr::ONE], expected_one);
        // 2
        let expected_two = Fr::from_str(
            "9632004710537414903275898870712812796867229507472840228295932832943785232633",
        )
        .unwrap();
        test_hasher(&[Fr::from(BigUint::from(2u8))], expected_two);
        // 0, 1, 2
        let expected_two = Fr::from_str(
            "21739021971472524335152491270920095773040444510968189350907442466992269802900",
        )
        .unwrap();
        test_hasher(
            &[Fr::from(0), Fr::ONE, Fr::from(BigUint::from(2u8))],
            expected_two,
        );
    }
}
