use std::sync::LazyLock;

use bincode::{
    config::{
        Bounded, FixintEncoding, LittleEndian, RejectTrailing, WithOtherEndian,
        WithOtherIntEncoding, WithOtherLimit, WithOtherTrailing,
    },
    Options as _,
};

// Type composition is cool but also makes naming types a bit awkward
pub type BincodeOptions = WithOtherTrailing<
    WithOtherIntEncoding<
        WithOtherLimit<WithOtherEndian<bincode::DefaultOptions, LittleEndian>, Bounded>,
        FixintEncoding,
    >,
    RejectTrailing,
>;

// TODO: Remove this once we transition to smaller proofs
// Risc0 proofs are HUGE (220 Kb) and it's the only reason we need to have this
// limit so large
pub const DATA_LIMIT: u64 = 1 << 18; // Do not serialize/deserialize more than 256 KiB
pub static OPTIONS: LazyLock<BincodeOptions> = LazyLock::new(|| {
    bincode::DefaultOptions::new()
        .with_little_endian()
        .with_limit(DATA_LIMIT)
        .with_fixint_encoding()
        .reject_trailing_bytes()
});

// Serialization functions
use bytes::{BufMut as _, Bytes, BytesMut};
use serde::{de::DeserializeOwned, Serialize};

use crate::codec::{Error as WireError, Result};

/// Serialize an object directly into bytes
pub fn serialize<T: Serialize>(item: &T) -> Result<Bytes> {
    let size = OPTIONS
        .serialized_size(item)
        .map_err(|e| WireError::Serialize(Box::new(e)))?;

    let buf = BytesMut::with_capacity(size as usize);

    let mut writer = buf.writer();
    bincode::serialize_into(&mut writer, item).map_err(|e| WireError::Serialize(Box::new(e)))?;

    Ok(writer.into_inner().freeze())
}

/// Get the serialized size of an object without actually serializing it
pub fn serialized_size<T: Serialize>(item: &T) -> Result<u64> {
    OPTIONS
        .serialized_size(item)
        .map_err(|e| WireError::Serialize(Box::new(e)))
}

/// Deserialize an object directly from bytes
pub fn deserialize<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    OPTIONS
        .deserialize(data)
        .map_err(|e| WireError::Deserialize(Box::new(e)))
}
