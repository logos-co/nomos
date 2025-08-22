#[cfg(feature = "deser")]
use std::str::FromStr;

#[cfg(feature = "deser")]
use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;

#[cfg(feature = "deser")]
pub mod deserialize;

#[cfg(feature = "deser")]
pub use deserialize::InputDeser;

#[derive(Copy, Clone)]
pub struct Input<E: Pairing>(<E as Pairing>::ScalarField);

impl<E: Pairing> Input<E> {
    pub const fn new(value: E::ScalarField) -> Self {
        Self(value)
    }
}

impl<E: Pairing> Input<E> {
    pub const fn into_inner(self) -> E::ScalarField {
        self.0
    }
}
impl<E: Pairing> AsRef<E::ScalarField> for Input<E> {
    fn as_ref(&self) -> &E::ScalarField {
        &self.0
    }
}

#[cfg(feature = "deser")]
impl TryFrom<InputDeser> for Input<Bn254> {
    type Error = <<Bn254 as Pairing>::ScalarField as FromStr>::Err;

    fn try_from(value: InputDeser) -> Result<Self, Self::Error> {
        Ok(Self(<Bn254 as Pairing>::ScalarField::from_str(
            value.0.as_str(),
        )?))
    }
}

#[cfg(feature = "deser")]
impl From<&Input<Bn254>> for InputDeser {
    fn from(value: &Input<Bn254>) -> Self {
        Self(value.0.to_string())
    }
}
