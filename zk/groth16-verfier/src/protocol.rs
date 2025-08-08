#[cfg(feature = "serde")]
use serde::Deserialize;

#[cfg_attr(feature = "serde", derive(Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Protocol {
    Groth16,
}

impl AsRef<str> for Protocol {
    fn as_ref(&self) -> &str {
        match self {
            Self::Groth16 => "groth16",
        }
    }
}
