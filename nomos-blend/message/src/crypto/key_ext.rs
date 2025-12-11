use ed25519_dalek::SigningKey;
use key_management_system_keys::keys::{Ed25519PublicKey, UnsecuredEd25519Key};
use nomos_blend_crypto::keys::{X25519PrivateKey, X25519PublicKey};
use nomos_utils::blake_rng::{BlakeRng, SeedableRng as _};
use zeroize::ZeroizeOnDrop;

// This extension trait must go here instead of `nomos-blend-crypto` because
// else we would have a circular dependency between that and
// `key-management-system-keys`. Also, these extension functions are mostly used
// in this crate, so it makes most sense for them to be defined here.
pub trait Ed25519SecretKeyExt: ZeroizeOnDrop {
    fn generate() -> Self;
    fn derive_x25519(&self) -> X25519PrivateKey;
}

impl Ed25519SecretKeyExt for UnsecuredEd25519Key {
    fn generate() -> Self {
        SigningKey::generate(&mut BlakeRng::from_entropy()).into()
    }

    fn derive_x25519(&self) -> X25519PrivateKey {
        self.as_ref().to_scalar_bytes().into()
    }
}

pub(crate) trait Ed25519PublicKeyExt {
    fn derive_x25519(&self) -> X25519PublicKey;
}

impl Ed25519PublicKeyExt for Ed25519PublicKey {
    fn derive_x25519(&self) -> X25519PublicKey {
        self.to_montgomery().to_bytes().into()
    }
}
