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
pub struct PositiveF64(
    #[cfg_attr(
        feature = "serde",
        serde(deserialize_with = "serde::deserialize_positive")
    )]
    NonNegativeF64,
);

impl PositiveF64 {
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0.get()
    }
}

impl Deref for PositiveF64 {
    type Target = f64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PositiveF64 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<f64> for PositiveF64 {
    type Error = ();

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        let non_negative = NonNegativeF64::try_from(value)?;
        Self::try_from(non_negative)
    }
}

impl TryFrom<NonNegativeF64> for PositiveF64 {
    type Error = ();

    fn try_from(value: NonNegativeF64) -> Result<Self, Self::Error> {
        if value.get() == 0.0 {
            Err(())
        } else {
            Ok(Self(value))
        }
    }
}

#[cfg(feature = "serde")]
mod serde {
    use serde::Deserialize as _;

    use crate::math::{NonNegativeF64, PositiveF64};

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

    pub(super) fn deserialize_positive<'de, Deserializer>(
        deserializer: Deserializer,
    ) -> Result<NonNegativeF64, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let inner = NonNegativeF64::deserialize(deserializer)?;
        PositiveF64::try_from(inner)
            .map_err(|()| serde::de::Error::custom("Deserialized f64 must be positive."))?;
        Ok(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_negative_basic() {
        assert!(NonNegativeF64::try_from(0.0).is_ok());
        assert!(NonNegativeF64::try_from(1.0).is_ok());
        assert!(NonNegativeF64::try_from(-0.1).is_err());
    }

    #[test]
    fn positive_basic() {
        assert!(PositiveF64::try_from(0.1).is_ok());
        assert!(PositiveF64::try_from(0.0).is_err());
        assert!(PositiveF64::try_from(-1.0).is_err());
    }

    #[cfg(feature = "serde")]
    mod serde_tests {
        use ::serde::Deserialize;

        use super::*;

        #[derive(Debug, Deserialize)]
        struct NonNegative {
            value: NonNegativeF64,
        }
        #[derive(Debug, Deserialize)]
        struct Positive {
            value: PositiveF64,
        }

        #[test]
        fn deser_non_negativef64() {
            let ok: NonNegative = serde_json::from_str(r#"{"value": 0.1}"#).unwrap();
            assert!((*ok.value - 0.1).abs() < f64::EPSILON);

            assert!(serde_json::from_str::<NonNegative>(r#"{"value": 0.0}"#).is_ok());
            assert!(serde_json::from_str::<NonNegative>(r#"{"value": -1.0}"#).is_err());
        }

        #[test]
        fn deser_positivef64() {
            let ok: Positive = serde_json::from_str(r#"{"value": 0.1}"#).unwrap();
            assert!((*ok.value - 0.1).abs() < f64::EPSILON);

            assert!(serde_json::from_str::<Positive>(r#"{"value": 0.0}"#).is_err());
            assert!(serde_json::from_str::<Positive>(r#"{"value": -1.0}"#).is_err());
        }
    }
}
