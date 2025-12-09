pub mod merkle;

use groth16::Fr;
use zeroize::Zeroize;

#[derive(Zeroize, Copy, Clone)]
pub struct SlotSecret(Fr);

impl AsRef<Fr> for SlotSecret {
    fn as_ref(&self) -> &Fr {
        &self.0
    }
}

impl From<Fr> for SlotSecret {
    fn from(fr: Fr) -> Self {
        Self(fr)
    }
}

pub const MAX_TREE_DEPTH: u8 = 25;
