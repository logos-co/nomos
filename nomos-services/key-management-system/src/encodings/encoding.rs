#![allow(
    dead_code,
    reason = "EncodingAdapter is only referenced via a blanket impl. The compiler treats it as unused, but it isn’t. This annotation will be removed once the trait is used directly."
)]

/// Marker trait for encodings.
///
/// Used for type safety and auto-implementing [`EncodingAdapter`].
pub trait Encoding {}

/// Marker trait for encoding adapters.
///
/// Requires the ability to convert between [`Self`] and [`TargetEncoding`].
pub trait EncodingAdapter<TargetEncoding: Encoding>:
    Encoding + TryFrom<TargetEncoding> + Into<TargetEncoding>
{
}

/// Auto-implement `EncodingAdapter` for `Encoding` when bounds are satisfied.
impl<AdapteeEncoding, TargetEncoding> EncodingAdapter<TargetEncoding> for AdapteeEncoding
where
    AdapteeEncoding: Encoding + TryFrom<TargetEncoding> + Into<TargetEncoding>,
    TargetEncoding: Encoding,
{
}
