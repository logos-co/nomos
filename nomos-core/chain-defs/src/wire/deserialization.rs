use serde::{de::DeserializeOwned, Deserialize};

use crate::wire::{
    bincode::{BincodeDeserializer, OPTIONS},
    Error,
};

pub struct Deserializer<'de> {
    inner: BincodeDeserializer<'de>,
}
impl<'de> Deserializer<'de> {
    pub fn get_deserializer(&mut self) -> impl serde::Deserializer<'de> + '_ {
        &mut self.inner
    }

    pub fn deserialize<T: Deserialize<'de>>(&mut self) -> crate::wire::Result<T> {
        <T>::deserialize(&mut self.inner).map_err(Error::Deserialize)
    }
}
/// Return a deserializer for wire format
///
/// We only operator on in-memory slices as to abstract
/// any underlying protocol. See <https://sans-io.readthedocs.io/how-to-sans-io.html>
#[must_use]
pub fn deserializer(data: &[u8]) -> Deserializer<'_> {
    Deserializer {
        inner: bincode::de::Deserializer::from_slice(data, *OPTIONS),
    }
}
/// Deserialize an object directly
pub fn deserialize<T: DeserializeOwned>(item: &[u8]) -> crate::wire::Result<T> {
    deserializer(item).deserialize()
}
