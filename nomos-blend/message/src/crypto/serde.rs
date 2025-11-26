use serde::{Deserialize as _, Deserializer, de::Error as _};

/// [`Ed25519PublicKey`] <> Hex string serialization/deserialization
pub mod ed25519_pubkey_hex {
    use nomos_blend_crypto::keys::{ED25519_PUBLIC_KEY_SIZE, Ed25519PublicKey};
    use serde::{Deserializer, Serialize as _, Serializer, de::Error as _};

    use crate::crypto::serde::deserialize_hex_to_key;

    pub fn serialize<S>(key: &Ed25519PublicKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(key.as_bytes()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Ed25519PublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ed25519PublicKey::from_bytes(&deserialize_hex_to_key::<ED25519_PUBLIC_KEY_SIZE, _>(
            deserializer,
        )?)
        .map_err(D::Error::custom)
    }
}

fn deserialize_hex_to_key<'de, const KEY_SIZE: usize, D>(
    deserializer: D,
) -> Result<[u8; KEY_SIZE], D::Error>
where
    D: Deserializer<'de>,
{
    hex::decode(String::deserialize(deserializer)?)
        .map_err(|e| D::Error::custom(format!("Invalid hex: {e}")))?
        .try_into()
        .map_err(|_| D::Error::custom("Invalid key length"))
}
