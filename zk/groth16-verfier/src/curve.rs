use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;
#[cfg(feature = "serde")]
use serde::Deserialize;

#[cfg_attr(feature = "serde", derive(Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Curve {
    BN128,
    BN254,
}

trait ToCurve: Pairing {
    fn to_curve(&self) -> Curve;
}

impl ToCurve for Bn254 {
    fn to_curve(&self) -> Curve {
        Curve::BN254
    }
}
