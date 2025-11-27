use ed25519_dalek::{SigningKey, ed25519::signature::Signer as _};
use key_management_system_keys::keys::{ED25519_SECRET_KEY_SIZE, UnsecuredEd25519Key};
use nomos_blend_crypto::{
    keys::{Ed25519PublicKey, X25519PrivateKey},
    signatures::Signature,
};
use nomos_utils::blake_rng::{BlakeRng, SeedableRng as _};
use zeroize::ZeroizeOnDrop;

// This extension trait must go here instead of `nomos-blend-crypto` because
// else we would have a circular dependency between that and
// `key-management-system-keys`. Also, these extension functions are mostly used
// in this crate, so it makes most sense for them to be defined here.
pub trait Ed25519SecretKeyExt: ZeroizeOnDrop {
    fn generate() -> Self;
    fn from_bytes(bytes: [u8; ED25519_SECRET_KEY_SIZE]) -> Self;
    fn public_key(&self) -> Ed25519PublicKey;
    fn sign(&self, message: &[u8]) -> Signature;
    fn derive_x25519(&self) -> X25519PrivateKey;
    fn as_bytes(&self) -> &[u8; ED25519_SECRET_KEY_SIZE];
}

impl Ed25519SecretKeyExt for UnsecuredEd25519Key {
    fn generate() -> Self {
        SigningKey::generate(&mut BlakeRng::from_entropy()).into()
    }

    fn from_bytes(bytes: [u8; ED25519_SECRET_KEY_SIZE]) -> Self {
        SigningKey::from_bytes(&bytes).into()
    }

    fn public_key(&self) -> Ed25519PublicKey {
        self.as_ref().verifying_key()
    }

    fn sign(&self, message: &[u8]) -> Signature {
        self.as_ref().sign(message).into()
    }

    fn derive_x25519(&self) -> X25519PrivateKey {
        self.as_ref().to_scalar_bytes().into()
    }

    fn as_bytes(&self) -> &[u8; ED25519_SECRET_KEY_SIZE] {
        self.as_ref().as_bytes()
    }
}
