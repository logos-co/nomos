use zeroize::ZeroizeOnDrop;

/// A key that can be used within the Key Management Service.
pub trait SecuredKey: ZeroizeOnDrop {
    type Payload;
    type Signature;
    type PublicKey;
    type Error;

    fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error>;
    fn as_public_key(&self) -> Self::PublicKey;
}
