#[cfg(feature = "deser")]
use serde::Deserialize;

#[cfg_attr(feature = "deser", derive(Deserialize))]
#[cfg_attr(feature = "deser", serde(rename_all = "lowercase"))]
pub enum Curve {
    BN128,
    BN254,
}
