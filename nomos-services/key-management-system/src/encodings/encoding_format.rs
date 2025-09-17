#![allow(
    dead_code,
    reason = "EncodingFormatAdapter is only referenced via a blanket impl. The compiler treats it as unused, but it isn’t. This annotation will be removed once the trait is used directly."
)]

use nomos_utils::convert::TryFromRef;

/// Marker trait for encoding formats.
///
/// Used for type safety and auto-implementing [`EncodingFormatAdapter`].
pub trait EncodingFormat {}

/// Marker trait for encoding formats that can adapt to another encoding format.
///
/// Requires the ability to convert between [`Self`] and
/// [`TargetEncodingFormat`].
pub trait EncodingFormatAdapter<'a, TargetEncodingFormat: EncodingFormat + 'a>:
    EncodingFormat + TryFromRef<'a, TargetEncodingFormat> + Into<TargetEncodingFormat>
{
}

/// Auto-implement the [`EncodingFormatAdapter`] trait for any type that
/// implements the required traits.
impl<'a, AdapteeEncodingFormat, TargetEncodingFormat>
    EncodingFormatAdapter<'a, TargetEncodingFormat> for AdapteeEncodingFormat
where
    AdapteeEncodingFormat:
        EncodingFormat + TryFromRef<'a, TargetEncodingFormat> + Into<TargetEncodingFormat>,
    TargetEncodingFormat: EncodingFormat + 'a,
{
}
