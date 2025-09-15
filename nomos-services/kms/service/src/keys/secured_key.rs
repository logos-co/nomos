use crate::{
    encodings::{Encoding, EncodingAdapter},
    keys::KeyError,
};

/// A key that can be used within the Key Management Service.
pub trait SecuredKey {
    type Encoding: Encoding;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, KeyError>;
}

/// A trait for keys that can sign data in an alternative encoding to their
/// native encoding.
///
/// This trait extends [`SecuredKey`] and allows a key to accept data in an
/// [`AdaptedEncoding`], as long as the key's native encoding implements
/// [`EncodingAdapter<AdaptedEncoding>`].
///
/// The input data in [`AdaptedEncoding`] is first converted to the key's native
/// encoding, then signed using the key and, finally, the signature is converted
/// back to [`AdaptedEncoding`].
///
/// # Type Parameters
///
/// - `AdaptedEncoding`: The alternative encoding type, which must implement
///   [`Encoding`].
///
/// # Example
///
/// ```ignore
/// let signature = key.sign_adapted(adapted_data)?;
/// ```
///
/// # Errors
///
/// Returns a [`KeyError`] if the conversion or signing fails.
pub trait SecuredKeyAdapter<AdaptedEncoding>: SecuredKey
where
    AdaptedEncoding: Encoding,
    Self::Encoding: EncodingAdapter<AdaptedEncoding>,
{
    fn sign_adapted(&self, data: AdaptedEncoding) -> Result<AdaptedEncoding, KeyError>;
}

/// Automatically implements [`SecuredKeyAdapter`] for any [`SecuredKey`] whose
/// encoding supports adaptation from [`AdaptedEncoding`].
impl<Key, AdaptedEncoding> SecuredKeyAdapter<AdaptedEncoding> for Key
where
    AdaptedEncoding: Encoding,
    Key: SecuredKey<Encoding: EncodingAdapter<AdaptedEncoding>>,
{
    fn sign_adapted(&self, data: AdaptedEncoding) -> Result<AdaptedEncoding, KeyError> {
        let payload = Self::Encoding::try_from(data).map_err(KeyError::from)?;
        self.sign(payload).map(Self::Encoding::into)
    }
}
