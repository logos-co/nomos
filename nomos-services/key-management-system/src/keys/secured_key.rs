use zeroize::ZeroizeOnDrop;

/// A key that can be used within the Key Management Service.
pub trait SecuredKey<Data>: ZeroizeOnDrop {
    type Signature;
    type PublicKey;
    type Error;

    fn sign(&self, data: &Data) -> Result<Self::Signature, Self::Error>;
    fn as_public_key(&self) -> Self::PublicKey;
}
