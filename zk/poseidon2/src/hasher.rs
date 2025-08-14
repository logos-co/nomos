use ark_bn254::Fr;
use num_bigint::BigUint;

use crate::{Poseidon2Bn254, Poseidon2Bn254Params, Poseidon2SpongeBn254};

pub struct Poseidon2Hasher {
    state: [Fr; 3],
}
impl Poseidon2Hasher {
    #[must_use]
    pub fn new() -> Self {
        let initial =
            BigUint::from(2u64).pow(64) + BigUint::from(256u64).pow(3) + BigUint::from(1u64);
        let state = [Fr::from(0u64), Fr::from(0u64), Fr::from(initial)];
        Self { state }
    }

    fn update_one(&mut self, input: &[u8]) {
        let input = Fr::from(BigUint::from_bytes_be(input));
        self.state[2] = &self.state[2] + &input;
        Poseidon2Bn254::permute_mut::<Poseidon2Bn254Params, 3>(&mut self.state);
    }

    pub fn update(&mut self, input: &[u8]) {
        for chunk in input.chunks(256) {
            self.update_one(chunk);
        }
    }

    pub fn finalize(&mut self) -> Fr {
        self.state[2]
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ark_ff::BigInteger;

    use super::*;

    #[test]
    fn hash() {
        let mut hasher = Poseidon2Hasher::new();
        hasher.update(0usize.to_be_bytes().as_ref());
        let result = hasher.finalize();
        let expected = Fr::from(
            BigUint::from_str(
                "10421270574331324177348727421681713543544081230097403733504870033499419662089",
            )
            .unwrap(),
        );
        assert_eq!(result, expected);
        println!("{:x?}", result.0.to_bytes_be());
    }
}
