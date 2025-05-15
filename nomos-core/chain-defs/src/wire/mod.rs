//! Serializer for wire formats.
// TODO: we're using bincode for now, but might need strong guarantees about
// the underlying format in the future for standardization.
pub(crate) mod bincode;
pub mod deserialization;
pub mod errors;
pub mod serialization;
// Exports
pub use deserialization::{deserialize, deserializer};
pub use errors::Error;
pub use serialization::{serialize, serializer, serializer_into_buffer};
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use serde::{Deserialize as _, Serialize as _};

    use super::*;

    #[test]
    fn serialize_deserialize() {
        let tmp = String::from("much wow, very cool");
        let mut buf = Vec::new();
        serializer(&mut buf).serialize_into(&tmp).unwrap();
        let deserialized = deserializer(&buf).deserialize::<String>().unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn serialize_deserialize_slice() {
        let tmp = String::from("much wow, very cool");
        let mut buf = vec![0; 1024];
        serializer_into_buffer(&mut buf)
            .serialize_into(&tmp)
            .unwrap();
        let deserialized = deserializer(&buf).deserialize::<String>().unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn serialize_deserialize_owned() {
        let tmp = String::from("much wow, very cool");
        let serialized = serialize(&tmp).unwrap();
        let deserialized = deserializer(&serialized).deserialize::<String>().unwrap();
        assert_eq!(tmp, deserialized);
    }

    #[test]
    fn serialize_deserialize_inner() {
        let tmp = String::from("much wow, very cool");
        let mut buf = Vec::new();
        let mut serializer = serializer(&mut buf);
        tmp.serialize(serializer.get_serializer()).unwrap();
        let mut deserializer = deserializer(&buf);
        let deserialized = <String>::deserialize(deserializer.get_deserializer()).unwrap();
        assert_eq!(tmp, deserialized);
    }
}
