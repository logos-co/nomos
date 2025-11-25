use key_management_system_keys::keys::Ed25519Key;
use nomos_blend_crypto::keys::X25519PrivateKey;

pub trait Ed25519SecretKeyExt {
    fn derive_x25519(&self) -> X25519PrivateKey;
}

impl Ed25519SecretKeyExt for Ed25519Key {
    fn derive_x25519(&self) -> X25519PrivateKey {
        self.as_ref().to_scalar_bytes().into()
    }
}
