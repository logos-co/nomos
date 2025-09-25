//! Serializer for wire formats.
// TODO: we're using bincode for now, but might need strong guarantees about
// the underlying format in the future for standardization.
pub(crate) mod bincode;
pub mod errors;
use std::error::Error as StdError;

use bincode::{deserialize, serialize, serialized_size};
use bytes::Bytes;
pub use errors::Error;
use serde::{Serialize, de::DeserializeOwned};
pub type Result<T> = std::result::Result<T, Error>;

/// Unified serialization trait for all wire and storage operations
pub trait SerdeOp {
    type Error: StdError;
    /// Serialize a value to bytes using wire format
    fn serialize<T: Serialize>(value: &T) -> Result<Bytes>;
    /// Deserialize bytes to a value using wire format
    fn deserialize<T: DeserializeOwned>(data: &[u8]) -> Result<T>;
    /// Get the serialized size of a value without actually serializing it
    fn serialized_size<T: Serialize>(value: &T) -> Result<u64>;
}

impl<S> SerdeOp for S
where
    S: Serialize + DeserializeOwned,
{
    type Error = Error;

    fn serialize<T: Serialize>(value: &T) -> Result<Bytes> {
        serialize(value)
    }

    fn deserialize<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
        deserialize(data)
    }

    fn serialized_size<T: Serialize>(value: &T) -> Result<u64> {
        serialized_size(value)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn serialize_deserialize() {
        let tmp = String::from("much wow, very cool");
        let serialized = <String as SerdeOp>::serialize(&tmp).unwrap();
        let deserialized: String = <String as SerdeOp>::deserialize(&serialized).unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn serialize_deserialize_owned() {
        let tmp = String::from("much wow, very cool");
        let serialized = <String as SerdeOp>::serialize(&tmp).unwrap();
        let deserialized: String = <String as SerdeOp>::deserialize(&serialized).unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn test_serialized_size() {
        let tmp = String::from("test");
        let size = <String as SerdeOp>::serialized_size(&tmp).unwrap();
        let serialized = <String as SerdeOp>::serialize(&tmp).unwrap();
        assert_eq!(size as usize, serialized.len());
    }
}
