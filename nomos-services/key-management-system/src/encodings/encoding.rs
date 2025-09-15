use crate::encodings::EncodingError;

/// Marker trait for encodings.
///
/// Used for type safety and auto-implementing [`EncodingAdapter`].
pub trait Encoding {}

/// Marker trait for encoding adapters.
///
/// Requires the ability to convert between [`Self`] and [`AdaptedEncoding`].
pub trait EncodingAdapter<AdaptedEncoding: Encoding>:
    Encoding + TryFrom<AdaptedEncoding, Error = EncodingError> + Into<AdaptedEncoding>
{
}

/// Auto-implement `EncodingAdapter` for `Encoding` when bounds are satisfied.
impl<AdaptedEncoding, AdapteeEncoding> EncodingAdapter<AdapteeEncoding> for AdaptedEncoding
where
    AdaptedEncoding:
        Encoding + TryFrom<AdapteeEncoding, Error = EncodingError> + Into<AdapteeEncoding>,
    AdapteeEncoding: Encoding,
{
}
