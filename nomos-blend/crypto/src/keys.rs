use ed25519_dalek::VerifyingKey;

pub type Ed25519PublicKey = VerifyingKey;
pub const ED25519_PUBLIC_KEY_LENGTH: usize = ed25519_dalek::PUBLIC_KEY_LENGTH;
