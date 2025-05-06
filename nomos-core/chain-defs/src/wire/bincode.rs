use std::sync::LazyLock;

use bincode::{
    config::{
        Bounded, FixintEncoding, LittleEndian, RejectTrailing, WithOtherEndian,
        WithOtherIntEncoding, WithOtherLimit, WithOtherTrailing,
    },
    de::read::SliceReader,
    Error, ErrorKind, Options as _,
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

pub type BincodeDeserializer<'de> = bincode::Deserializer<SliceReader<'de>, BincodeOptions>;
pub type BincodeSerializer<T> = bincode::Serializer<T, BincodeOptions>;

pub fn clone_bincode_error(error: &Error) -> ErrorKind {
    match error.as_ref() {
        ErrorKind::Io(error) => ErrorKind::Io(std::io::Error::new(error.kind(), error.to_string())),
        ErrorKind::InvalidUtf8Encoding(error) => ErrorKind::InvalidUtf8Encoding(*error),
        ErrorKind::InvalidBoolEncoding(bool) => ErrorKind::InvalidBoolEncoding(*bool),
        ErrorKind::InvalidCharEncoding => ErrorKind::InvalidCharEncoding,
        ErrorKind::InvalidTagEncoding(tag) => ErrorKind::InvalidTagEncoding(*tag),
        ErrorKind::DeserializeAnyNotSupported => ErrorKind::DeserializeAnyNotSupported,
        ErrorKind::SizeLimit => ErrorKind::SizeLimit,
        ErrorKind::SequenceMustHaveLength => ErrorKind::SequenceMustHaveLength,
        ErrorKind::Custom(custom) => ErrorKind::Custom(custom.clone()),
    }
}
