#![allow(
    dead_code,
    reason = "SecureKeyAdapter is only referenced via a blanket impl. The compiler treats it as unused, but it isn’t. This annotation will be removed once the trait is used directly."
)]

use zeroize::ZeroizeOnDrop;

use crate::encodings::{EncodingFormat, EncodingFormatAdapter};

/// A key that can be used within the Key Management Service.
pub trait SecuredKey: ZeroizeOnDrop {
    type EncodingFormat: EncodingFormat;
    type Error;

    fn sign(&self, data: Self::EncodingFormat) -> Result<Self::EncodingFormat, Self::Error>;
    fn as_pk(&self) -> Self::EncodingFormat;
}

/// A trait for keys that can sign data in an alternative encoding to their
/// native encoding.
///
/// This trait extends [`SecuredKey`] and allows a key to accept data in an
/// [`TargetEncodingFormat`], as long as the key's native encoding implements
/// [`EncodingFormatAdapter<TargetEncodingFormat>`].
///
/// The input data in [`TargetEncodingFormat`] is first converted to the key's
/// native encoding, then signed using the key and, finally, the signature is
/// converted back to [`TargetEncodingFormat`].
///
/// # Type Parameters
///
/// - [`TargetEncodingFormat`]: The alternative encoding type, which must
///   implement [`EncodingFormat`].
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
pub trait SecuredKeyAdapter<TargetEncodingFormat>: SecuredKey
where
    TargetEncodingFormat: EncodingFormat,
    Self::EncodingFormat: EncodingFormatAdapter<TargetEncodingFormat>,
{
    type TargetError;

    fn sign_adapted(
        &self,
        data: TargetEncodingFormat,
    ) -> Result<TargetEncodingFormat, Self::TargetError>;
}

/// Automatically implements [`SecuredKeyAdapter`] for any [`SecuredKey`] whose
/// encoding supports adaptation from [`TargetEncodingFormat`].
impl<Key, TargetEncodingFormat> SecuredKeyAdapter<TargetEncodingFormat> for Key
where
    TargetEncodingFormat: EncodingFormat,
    Key: SecuredKey,
    <Key as SecuredKey>::EncodingFormat: EncodingFormatAdapter<TargetEncodingFormat>,
    <<Key as SecuredKey>::EncodingFormat as TryFrom<TargetEncodingFormat>>::Error: Into<Key::Error>,
{
    type TargetError = Key::Error;

    fn sign_adapted(
        &self,
        data: TargetEncodingFormat,
    ) -> Result<TargetEncodingFormat, Self::TargetError> {
        let payload = Self::EncodingFormat::try_from(data).map_err(Into::into)?;
        self.sign(payload).map(Self::EncodingFormat::into)
    }
}
