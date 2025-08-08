use std::ops::Mul;

use ark_bn254::{Bn254, Fq2, G2Affine};
use ark_ec::{CurveGroup, pairing::Pairing};
#[cfg(feature = "deser")]
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[cfg(feature = "deser")]
use crate::proof::serde_proof::ProofDeser;

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

#[cfg(feature = "deser")]
use ark_bn254::{Fq, G1Affine};
#[cfg(feature = "deser")]
use ark_ff::PrimeField;
#[cfg(feature = "deser")]
use num_bigint::BigUint;

#[cfg(feature = "deser")]
fn try_biguint_as_g1(big_uint: BigUint) -> Option<G1Affine> {
    // Convert to field element
    let bytes = big_uint.to_bytes_le();
    let x = Fq::from_le_bytes_mod_order(&bytes);

    // Try to find a valid point with this x-coordinate
    // This might fail if the x doesn't correspond to a valid curve point
    G1Affine::get_point_from_x_unchecked(x, false)
        .or_else(|| G1Affine::get_point_from_x_unchecked(x, true))
}

#[cfg(feature = "deser")]
fn try_biguint_pair_as_g2(big_uint1: BigUint, big_uint2: BigUint) -> Option<G2Affine> {
    // Use two BigUints for real and imaginary parts of Fq2
    let bytes1 = big_uint1.to_bytes_le();
    let bytes2 = big_uint2.to_bytes_le();

    let real_part = Fq::from_le_bytes_mod_order(&bytes1);
    let imag_part = Fq::from_le_bytes_mod_order(&bytes2);

    let x = Fq2::new(real_part, imag_part);

    G2Affine::get_point_from_x_unchecked(x, false)
        .or_else(|| G2Affine::get_point_from_x_unchecked(x, true))
}

#[cfg(feature = "deser")]
impl<'de> Deserialize<'de> for Proof<Bn254> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: ProofDeser<Bn254> = Deserialize::deserialize(deserializer)?;
        Ok(value.into())
    }
}

#[cfg(feature = "deser")]
impl From<ProofDeser<Bn254>> for Proof<Bn254> {
    fn from(value: ProofDeser<Bn254>) -> Self {
        assert_eq!(value.protocol.as_ref(), "groth16");
        let ProofDeser {
            pi_a, pi_b, pi_c, ..
        } = value;
        let [b1, b2, b3] = pi_b;
        Self {
            pi_a: try_biguint_as_g1(pi_a).unwrap().into(),
            pi_b: try_biguint_pair_as_g2(b1, b2).unwrap().into(),
            pi_c: try_biguint_as_g1(pi_c).unwrap().into(),
        }
    }
}

#[cfg(feature = "deser")]
mod serde_proof {
    use std::{marker::PhantomData, str::FromStr};

    use ark_ec::{AffineRepr, CurveGroup};
    use num_bigint::BigUint;
    use serde::{Deserialize, Deserializer, de::Error};

    use super::*;
    use crate::{curve::Curve, protocol::Protocol};

    #[derive(Deserialize)]
    pub(crate) struct ProofDeser<E: Pairing> {
        pub protocol: Protocol,
        pub curve: Curve,
        pub pi_a: BigUint,
        pub pi_b: [BigUint; 3],
        pub pi_c: BigUint,
        #[serde(skip)]
        _pairing: PhantomData<*const E>,
    }
}

#[cfg(test)]
mod tests {
    use ark_bn254::Bn254;
    use serde_json::json;

    use crate::proof::Proof;

    #[cfg(feature = "deser")]
    #[test]
    fn deserialize() {
        let data = json!(
                    {
          "pi_a": [
            "8296175608850998036255335084231000907125502603097068078993517773809496732066",
            "8263160927867860156491312948728748265016489542834411322655068343855704802368",
            "1"
          ],
          "pi_b": [
            [
              "21630590412244703770464699084160733144935501859194730009968664948222752546282",
              "2360176260887090528387414040841390178721803616623769558861196687249493928600"
            ],
            [
              "19520030071777612089051083418787870247443252641482678846010900794231980067541",
              "10365922284519340998921178202220836853052351283418810378278857066381010824566"
            ],
            [
              "1",
              "0"
            ]
          ],
          "pi_c": [
            "6696664968468451496397455124742234961189848064077552976860754045639269197981",
            "6523385944235793127051945618289282151393577593495757596060209123245519772531",
            "1"
          ],
          "protocol": "groth16",
          "curve": "bn128"
        }
                );
        let proof: Proof<Bn254> = serde_json::from_value(data).unwrap();
    }
}
