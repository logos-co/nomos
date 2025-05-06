use bincode::Options as _;
use serde::Serialize;

use crate::wire::{
    bincode::{BincodeSerializer, OPTIONS},
    Error, Result,
};

pub struct Serializer<T> {
    inner: BincodeSerializer<T>,
}

impl<T: std::io::Write> Serializer<T> {
    pub fn get_serializer(&mut self) -> impl serde::Serializer + '_ {
        &mut self.inner
    }

    pub fn serialize_into<U: Serialize>(
        &mut self,
        item: &U,
    ) -> Result<<&mut BincodeSerializer<T> as serde::Serializer>::Ok> {
        item.serialize(&mut self.inner).map_err(Error::Serialize)
    }
}

/// Return a serializer for wire format.
///
/// We only operator on in-memory slices as to abstract
/// any underlying protocol. See <https://sans-io.readthedocs.io/how-to-sans-io.html>
pub fn serializer(buffer: &mut Vec<u8>) -> Serializer<&'_ mut Vec<u8>> {
    Serializer {
        inner: bincode::Serializer::new(buffer, *OPTIONS),
    }
}

/// Return a serializer for wire format that overwrites (but now grow) the
/// provided buffer.
///
/// We only operator on in-memory slices as to abstract
/// any underlying protocol. See <https://sans-io.readthedocs.io/how-to-sans-io.html>
pub fn serializer_into_buffer(buffer: &mut [u8]) -> Serializer<&'_ mut [u8]> {
    Serializer {
        inner: bincode::Serializer::new(buffer, *OPTIONS),
    }
}

/// Serialize an object directly into a vec
pub fn serialize<T: Serialize>(item: &T) -> Result<Vec<u8>> {
    let size = OPTIONS.serialized_size(item).map_err(Error::Serialize)?;
    let mut buf = Vec::with_capacity(size as usize);
    serializer(&mut buf).serialize_into(item)?;
    Ok(buf)
}
