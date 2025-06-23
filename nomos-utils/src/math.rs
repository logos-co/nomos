use core::ops::{Deref, DerefMut};

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub struct NonNegativeF64(
    #[cfg_attr(feature = "serde", serde(deserialize_with = "serde::deserialize"))] f64,
);

impl NonNegativeF64 {
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Deref for NonNegativeF64 {
    type Target = f64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NonNegativeF64 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<f64> for NonNegativeF64 {
    type Error = ();

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value >= 0f64 {
            Ok(Self(value))
        } else {
            Err(())
        }
    }
}

#[cfg(feature = "serde")]
mod serde {
    use serde::Deserialize as _;

    use crate::math::NonNegativeF64;

    pub(super) fn deserialize<'de, Deserializer>(
        deserializer: Deserializer,
    ) -> Result<f64, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let inner = f64::deserialize(deserializer)?;
        NonNegativeF64::try_from(inner).map_err(|()| {
            serde::de::Error::custom("Deserialized f64 does not contain a valid value.")
        })?;
        Ok(inner)
    }
}
