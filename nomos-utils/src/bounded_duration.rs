use std::{fmt::Display, marker::PhantomData};

use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{DeserializeAs, SerializeAs};
use time::Duration;

// TODO: This type can be fancier once const for generic types is implemented
// https://doc.rust-lang.org/beta/unstable-book/language-features/adt-const-params.html
// This means that we could add different bound from a const configuration
// checked on compile time.
/// This is a proxy type to be use to bound `Duration` deserialization checks.
///
/// It embeds the checks at type level. It implements the pertinent types from
/// the `serde_with` crate so it can be use within `serde_as` macros.
///
/// # Examples
/// ```rust
///     use serde_with::serde_as;
///     use time::Duration;
///     use nomos_utils::bounded_duration::{SECOND, MinimalBoundedDuration};
///
///     #[serde_as]
///     #[derive(serde::Serialize, serde::Deserialize)]
///     struct Foo {
///         #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
///         duration: Duration,
///     }
/// ```
// The `TimeTag` trait is not public, is just to tie up types for
// `MinimalBoundedDuration` and its implemented only by the supported types.
#[expect(private_bounds)]
#[derive(Serialize)]
pub struct MinimalBoundedDuration<const MIN_DURATION: usize, TAG: TimeTag> {
    duration: Duration,
    #[serde(skip_serializing)]
    _tag: PhantomData<*const TAG>,
}

pub(crate) trait TimeTag {
    fn inner() -> char;
}

pub struct BoundTag<const TAG: char> {}
impl<const TAG: char> TimeTag for BoundTag<TAG> {
    fn inner() -> char {
        TAG
    }
}

/// *Nanoseconds* tag type for `MinimalBoundedDuration`
pub type NANO = BoundTag<'n'>;
/// *Milliseconds* tag type for `MinimalBoundedDuration`
pub type MILLI = BoundTag<'l'>;
/// *Seconds* tag type for `MinimalBoundedDuration`
pub type SECOND = BoundTag<'s'>;
/// *Minute* tag type for `MinimalBoundedDuration`
pub type MINUTE = BoundTag<'m'>;
/// *Hour* tag type for `MinimalBoundedDuration`
pub type HOUR = BoundTag<'h'>;
/// *Day* tag type for `MinimalBoundedDuration`
pub type DAY = BoundTag<'d'>;

// we have a limitation to const types, so we better use the ones defined.
fn fill_duration_measure<T: TimeTag>() -> Result<String, impl Display> {
    match T::inner() {
        v @ ('d' | 'h' | 'm' | 's') => Ok(v.to_string()),
        'l' => Ok("ms".to_string()),
        'n' => Ok("ns".to_string()),
        other => Err(format!("'{other}' measure not supported")),
    }
}

impl<const MIN_DURATION: usize, TAG: TimeTag> SerializeAs<Duration>
    for MinimalBoundedDuration<MIN_DURATION, TAG>
{
    fn serialize_as<S>(source: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        source.serialize(serializer)
    }
}

impl<'de, const MIN_DURATION: usize, TAG: TimeTag> DeserializeAs<'de, Duration>
    for MinimalBoundedDuration<MIN_DURATION, TAG>
{
    fn deserialize_as<D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let duration: Self = Self::deserialize(deserializer)?;
        Ok(duration.duration)
    }
}

impl<'de, const MIN_DURATION: usize, TAG: TimeTag> Deserialize<'de>
    for MinimalBoundedDuration<MIN_DURATION, TAG>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Duration = Duration::deserialize(deserializer)?;
        let reformat_measure = fill_duration_measure::<TAG>().map_err(Error::custom)?;
        let parsed_duration =
            humantime::parse_duration(format!("{MIN_DURATION}{reformat_measure}").as_str())
                .map_err(Error::custom)?;
        let min_duration: Duration = Duration::new(
            parsed_duration
                .as_secs()
                .try_into()
                .map_err(Error::custom)?,
            parsed_duration
                .subsec_nanos()
                .try_into()
                .map_err(Error::custom)?,
        );
        if value < min_duration {
            return Err(Error::custom(format!(
                "Minimal duration is {min_duration} but got {value}"
            )));
        }
        Ok(Self {
            duration: value,
            _tag: PhantomData,
        })
    }
}

impl<const MIN_DURATION: usize, TAG: TimeTag> From<MinimalBoundedDuration<MIN_DURATION, TAG>>
    for Duration
{
    fn from(value: MinimalBoundedDuration<MIN_DURATION, TAG>) -> Self {
        value.duration
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use serde_with::serde_as;
    use time::Duration;

    use crate::bounded_duration::{MinimalBoundedDuration, DAY, SECOND};

    #[test]
    fn success_deserialize() {
        let json_value = json!("1.0");
        let value = serde_json::to_value(json_value).unwrap();
        let _duration: MinimalBoundedDuration<1, SECOND> = serde_json::from_value(value).unwrap();
    }

    #[test]
    #[should_panic]
    fn fail_deserialize_with_type_bound() {
        let value = serde_json::to_value(Duration::seconds(1)).unwrap();

        let _duration: MinimalBoundedDuration<1, DAY> = serde_json::from_value(value).unwrap();
    }

    #[serde_as]
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Foo {
        #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
        duration: Duration,
    }

    #[test]
    fn deserialize_proxy_type_success() {
        let foo = Foo {
            duration: Duration::seconds(10),
        };
        let _foo: Foo = serde_json::from_value(serde_json::to_value(&foo).unwrap()).unwrap();
    }

    #[test]
    #[should_panic]
    fn deserialize_proxy_type_fails() {
        let foo = Foo {
            duration: Duration::milliseconds(10),
        };
        let _foo: Foo = serde_json::from_value(serde_json::to_value(&foo).unwrap()).unwrap();
    }
}
