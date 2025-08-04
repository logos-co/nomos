use crate::utils::serde_bytes_newtype;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SecretKey([u8; 16]);
serde_bytes_newtype!(SecretKey, 16);

impl SecretKey {
    #[must_use]
    pub const fn new(key: [u8; 16]) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    #[must_use]
    pub fn to_public_key(&self) -> PublicKey {
        unimplemented!("Conversion from SecretKey to PublicKey is not implemented yet")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);
serde_bytes_newtype!(PublicKey, 32);

impl PublicKey {
    #[must_use]
    pub const fn new(key: [u8; 32]) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<SecretKey> for PublicKey {
    fn from(secret: SecretKey) -> Self {
        secret.to_public_key()
    }
}

impl From<[u8; 16]> for SecretKey {
    fn from(key: [u8; 16]) -> Self {
        Self::new(key)
    }
}

impl From<[u8; 32]> for PublicKey {
    fn from(key: [u8; 32]) -> Self {
        Self::new(key)
    }
}

impl From<SecretKey> for [u8; 16] {
    fn from(secret: SecretKey) -> Self {
        secret.0
    }
}

impl From<PublicKey> for [u8; 32] {
    fn from(public: PublicKey) -> Self {
        public.0
    }
}
