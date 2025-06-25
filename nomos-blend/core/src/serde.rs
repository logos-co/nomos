use nomos_blend_message::crypto::KEY_SIZE;
use serde::{de::Error as _, Deserialize as _, Deserializer};

/// [`Ed25519PrivateKey`] <> Hex string hex/deserialization
pub mod ed25519_privkey_hex {
    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use serde::{Deserializer, Serialize as _, Serializer};

    use crate::serde::deserialize_hex_to_key;

    pub fn serialize<S>(key: &Ed25519PrivateKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(key.as_bytes()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Ed25519PrivateKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(deserialize_hex_to_key(deserializer)?.into())
    }
}

/// [`Ed25519PublicKey`] <> Hex string serialization/deserialization
pub mod ed25519_pubkey_hex {
    use nomos_blend_message::crypto::Ed25519PublicKey;
    use serde::{de::Error as _, Deserializer, Serialize as _, Serializer};

    use crate::serde::deserialize_hex_to_key;

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
        deserialize_hex_to_key(deserializer)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

fn deserialize_hex_to_key<'de, D>(deserializer: D) -> Result<[u8; KEY_SIZE], D::Error>
where
    D: Deserializer<'de>,
{
    hex::decode(String::deserialize(deserializer)?)
        .map_err(|e| D::Error::custom(format!("Invalid hex: {e}")))?
        .try_into()
        .map_err(|_| D::Error::custom("Invalid key length"))
}
