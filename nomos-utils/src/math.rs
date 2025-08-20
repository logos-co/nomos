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

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub struct NonZeroF64(
    #[cfg_attr(
        feature = "serde",
        serde(deserialize_with = "serde::deserialize_nonzero")
    )]
    f64,
);

impl NonZeroF64 {
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Deref for NonZeroF64 {
    type Target = f64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NonZeroF64 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<f64> for NonZeroF64 {
    type Error = ();

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value == 0f64 {
            Err(())
        } else {
            Ok(Self(value))
        }
    }
}

#[cfg(feature = "serde")]
mod serde {
    use serde::Deserialize as _;

    use crate::math::{NonNegativeF64, NonZeroF64};

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

    pub(super) fn deserialize_nonzero<'de, Deserializer>(
        deserializer: Deserializer,
    ) -> Result<f64, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let inner = f64::deserialize(deserializer)?;
        NonZeroF64::try_from(inner)
            .map_err(|()| serde::de::Error::custom("Deserialized f64 must be non-zero."))?;
        Ok(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_negative_f64() {
        assert!(NonNegativeF64::try_from(0.0).is_ok());
        assert!(NonNegativeF64::try_from(1.0).is_ok());
        assert!(NonNegativeF64::try_from(42.5).is_ok());
        assert!(NonNegativeF64::try_from(-1.0).is_err());
        assert!(NonNegativeF64::try_from(-0.1).is_err());
    }

    #[test]
    fn test_non_zero_f64() {
        assert!(NonZeroF64::try_from(1.0).is_ok());
        assert!(NonZeroF64::try_from(42.5).is_ok());
        assert!(NonZeroF64::try_from(-1.0).is_ok());
        assert!(NonZeroF64::try_from(-0.1).is_ok());
        assert!(NonZeroF64::try_from(0.0).is_err());
        assert!(NonZeroF64::try_from(-0.0).is_err());
    }

    #[test]
    fn test_non_zero_f64_deref() {
        let value = NonZeroF64::try_from(2.5).unwrap();
        assert!((*value - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_non_zero_f64_get() {
        let value = NonZeroF64::try_from(-2.5).unwrap();
        assert!((value.get() - (-2.5)).abs() < f64::EPSILON);
    }
}
