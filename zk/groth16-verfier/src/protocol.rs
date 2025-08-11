#[cfg(feature = "serde")]
use serde::Deserialize;

#[cfg_attr(feature = "deser", derive(Deserialize))]
#[cfg_attr(feature = "deser", serde(rename_all = "lowercase"))]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
