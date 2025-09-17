use key_management_system_macros::SimpleEncodingFormat;

type Inner = bytes::Bytes;

/// An encoding of arbitrary bytes.
#[derive(SimpleEncodingFormat, Clone)]
pub struct Bytes(Inner);

impl Bytes {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<Inner> for Bytes {
    fn from(value: Inner) -> Self {
        Self(value)
    }
}
