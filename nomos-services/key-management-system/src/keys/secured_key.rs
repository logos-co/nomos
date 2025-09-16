#![allow(
    dead_code,
    reason = "SecureKeyAdapter is only referenced via a blanket impl. The compiler treats it as unused, but it isn’t. This annotation will be removed once the trait is used directly."
)]

use crate::encodings::{Encoding, EncodingAdapter};

/// A key that can be used within the Key Management Service.
pub trait SecuredKey {
    type Encoding: Encoding;
    type Error;

    fn sign(&self, data: Self::Encoding) -> Result<Self::Encoding, Self::Error>;
    fn as_pk(&self) -> Self::Encoding;
}

/// A trait for keys that can sign data in an alternative encoding to their
/// native encoding.
///
/// This trait extends [`SecuredKey`] and allows a key to accept data in an
/// [`TargetEncoding`], as long as the key's native encoding implements
/// [`EncodingAdapter<TargetEncoding>`].
///
/// The input data in [`TargetEncoding`] is first converted to the key's native
/// encoding, then signed using the key and, finally, the signature is converted
/// back to [`TargetEncoding`].
///
/// # Type Parameters
///
/// - [`TargetEncoding`]: The alternative encoding type, which must implement
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
pub trait SecuredKeyAdapter<TargetEncoding>: SecuredKey
where
    TargetEncoding: Encoding,
    Self::Encoding: EncodingAdapter<TargetEncoding>,
{
    type TargetError;

    fn sign_adapted(&self, data: TargetEncoding) -> Result<TargetEncoding, Self::TargetError>;
}

/// Automatically implements [`SecuredKeyAdapter`] for any [`SecuredKey`] whose
/// encoding supports adaptation from [`TargetEncoding`].
impl<Key, TargetEncoding> SecuredKeyAdapter<TargetEncoding> for Key
where
    TargetEncoding: Encoding,
    Key: SecuredKey,
    <Key as SecuredKey>::Encoding: EncodingAdapter<TargetEncoding>,
    <<Key as SecuredKey>::Encoding as TryFrom<TargetEncoding>>::Error: Into<Key::Error>,
{
    type TargetError = Key::Error;

    fn sign_adapted(&self, data: TargetEncoding) -> Result<TargetEncoding, Self::TargetError> {
        let payload = Self::Encoding::try_from(data).map_err(Into::into)?;
        self.sign(payload).map(Self::Encoding::into)
    }
}
