use bincode::{
    config::{Configuration, Fixint, LittleEndian},
    error::DecodeError,
    serde::{decode_from_slice, encode_to_vec},
};
use serde::{de::DeserializeOwned, Serialize};

pub trait FixedSizeSerde: Copy + Serialize + DeserializeOwned {
    const SIZE: usize;

    fn to_bytes(&self) -> Vec<u8> {
        let vec = encode_to_vec(self, Self::bincode_config()).expect("Failed to serialize");
        assert_eq!(vec.len(), Self::SIZE);
        vec
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, FixedSizeSerdeError> {
        let (out, size): (Self, usize) = decode_from_slice(bytes, Self::bincode_config())
            .map_err(FixedSizeSerdeError::DeserializeError)?;
        assert_eq!(size, Self::SIZE);
        Ok(out)
    }

    fn bincode_config() -> Configuration<LittleEndian, Fixint> {
        bincode::config::standard()
            .with_fixed_int_encoding()
            .with_little_endian()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FixedSizeSerdeError {
    #[error("Failed to deserialize")]
    DeserializeError(DecodeError),
}
