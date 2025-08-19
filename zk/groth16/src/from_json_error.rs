use ark_bn254::{G1Affine, G2Affine};

use crate::utils::{StringifiedG1, StringifiedG2};

#[derive(Debug, thiserror::Error)]
pub enum FromJsonError {
    #[error("invalid protocol: {0}, expected: \"groth16\"")]
    WrongProtocol(String),
    #[error("could not parse G1 point, due to: {0:?}")]
    G1PointConversionError(<G1Affine as TryFrom<StringifiedG1>>::Error),
    #[error("could not parse G2 point, due to: {0:?}")]
    G2PointConversionError(<G2Affine as TryFrom<StringifiedG2>>::Error),
}
