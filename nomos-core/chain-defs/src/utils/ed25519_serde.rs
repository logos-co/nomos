use ed25519_dalek::VerifyingKey;
use serde::{Deserialize as _, Deserializer, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

/// Serialize bytes as hex string for human-readable formats, bytes for
/// binary formats
pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if serializer.is_human_readable() {
        serializer.serialize_str(&hex::encode(bytes))
    } else {
        serializer.serialize_bytes(bytes)
    }
}

/// Deserialize bytes from hex string for human-readable formats, bytes
/// for binary formats
pub fn deserialize<'de, const SIZE: usize, D>(deserializer: D) -> Result<[u8; SIZE], D::Error>
where
    D: Deserializer<'de>,
{
    if deserializer.is_human_readable() {
        let hex_str = String::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_str)
            .map_err(|e| serde::de::Error::custom(format!("Invalid hex string: {e}")))?;
        let array: [u8; SIZE] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Expected {SIZE} bytes"))?;
        Ok(array)
    } else {
        let bytes = <&[u8]>::deserialize(deserializer)?;
        let array: [u8; SIZE] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Expected {SIZE} bytes"))?;
        Ok(array)
    }
}

pub(crate) struct Ed25519Hex;

impl SerializeAs<VerifyingKey> for Ed25519Hex {
    fn serialize_as<S>(source: &VerifyingKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(source.as_bytes(), serializer)
    }
}

impl<'de> DeserializeAs<'de, VerifyingKey> for Ed25519Hex {
    fn deserialize_as<D>(deserializer: D) -> Result<VerifyingKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserialize::<32, D>(deserializer)?;
        VerifyingKey::from_bytes(&bytes)
            .map_err(|e| serde::de::Error::custom(format!("Invalid Ed25519 public key: {e}")))
    }
}

/// Module for use with #[serde(with = "`sig_hex`")] on `ed25519::Signature`
/// fields
pub(crate) mod sig_hex {
    use super::{Deserializer, Serializer};

    pub fn serialize<S>(sig: &ed25519::Signature, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::serialize(&sig.to_bytes(), serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519::Signature, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::deserialize::<64, D>(deserializer)
            .map(|bytes| ed25519::Signature::from_bytes(&bytes))
    }
}
