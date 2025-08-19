#[cfg(feature = "deser")]
use std::str::FromStr;

#[cfg(feature = "deser")]
use ark_bn254::Bn254;
use ark_ec::pairing::Pairing;

#[cfg(feature = "deser")]
pub mod deserialize;

#[cfg(feature = "deser")]
pub use deserialize::PublicInputDeser;

pub struct PublicInput<E: Pairing>(<E as Pairing>::ScalarField);

impl<E: Pairing> PublicInput<E> {
    pub const fn into_inner(self) -> E::ScalarField {
        self.0
    }
}
impl<E: Pairing> AsRef<E::ScalarField> for PublicInput<E> {
    fn as_ref(&self) -> &E::ScalarField {
        &self.0
    }
}

#[cfg(feature = "deser")]
impl TryFrom<PublicInputDeser> for PublicInput<Bn254> {
    type Error = <<Bn254 as Pairing>::ScalarField as FromStr>::Err;

    fn try_from(value: PublicInputDeser) -> Result<Self, Self::Error> {
        Ok(Self(<Bn254 as Pairing>::ScalarField::from_str(
            value.0.as_str(),
        )?))
    }
}
