use key_management_system_keys::keys::Ed25519Key;
use nomos_blend_crypto::keys::X25519PrivateKey;

// This extension trait must go here instead of `nomos-blend-crypto` because
// else we would have a circular dependency between that and
// `key-management-system-keys`. Also, these extension functions are mostly used
// in this crate, so it makes most sense for them to be defined here.
pub trait Ed25519SecretKeyExt {
    fn derive_x25519(&self) -> X25519PrivateKey;
}

impl Ed25519SecretKeyExt for Ed25519Key {
    fn derive_x25519(&self) -> X25519PrivateKey {
        self.as_ref().to_scalar_bytes().into()
    }
}
