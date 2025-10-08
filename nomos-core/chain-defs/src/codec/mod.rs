//! Serializer for wire formats.
// TODO: we're using bincode for now, but might need strong guarantees about
// the underlying format in the future for standardization.
pub(crate) mod bincode;
pub mod errors;
use std::error::Error as StdError;

use bytes::Bytes;
pub use errors::Error;
use serde::{Serialize, de::DeserializeOwned};
pub type Result<T> = std::result::Result<T, Error>;

/// Unified serialization trait for all wire and storage operations
pub trait SerdeOp: Serialize {
    type Error: StdError;

    fn to_bytes(&self) -> Result<Bytes>;

    fn from_bytes(data: &[u8]) -> Result<Self>
    where
        Self: DeserializeOwned;

    fn bytes_size(&self) -> Result<u64>;
}

impl<S: Serialize> SerdeOp for S {
    type Error = Error;

    fn to_bytes(&self) -> Result<Bytes> {
        bincode::serialize(self)
    }

    fn from_bytes(data: &[u8]) -> Result<Self>
    where
        Self: DeserializeOwned,
    {
        bincode::deserialize(data)
    }

    fn bytes_size(&self) -> Result<u64> {
        bincode::serialized_size(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_deserialize() {
        let tmp = String::from("much wow, very cool");
        let serialized = tmp.to_bytes().unwrap();
        let deserialized: String = String::from_bytes(&serialized).unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn serialize_deserialize_owned() {
        let tmp = String::from("much wow, very cool");
        let serialized = tmp.to_bytes().unwrap();
        let deserialized: String = String::from_bytes(&serialized).unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn test_serialized_size() {
        let tmp = String::from("test");
        let size = tmp.bytes_size().unwrap();
        let serialized = tmp.to_bytes().unwrap();
        assert_eq!(size as usize, serialized.len());
    }
}
