use crate::crypto::keys::{Ed25519PublicKey, KEY_SIZE};

pub mod prove;
pub mod verify;
pub use self::verify::Inputs as VerifyInputs;

type HalfEphemeralSigningKey = [u8; KEY_SIZE / 2];

fn split_ephemeral_signing_key(
    key: Ed25519PublicKey,
) -> (HalfEphemeralSigningKey, HalfEphemeralSigningKey) {
    let key_bytes = key.as_bytes();
    (
        key_bytes[0..(KEY_SIZE / 2)]
            .try_into()
            .expect("Ephemeral signing key must be exactly 32 bytes long."),
        key_bytes[(KEY_SIZE / 2)..]
            .try_into()
            .expect("Ephemeral signing key must be exactly 32 bytes long."),
    )
}
