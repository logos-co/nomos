use std::{
    any::type_name,
    fmt::{Display, Formatter},
};

type Inner = bytes::Bytes;

/// An encoding of arbitrary bytes.
#[derive(Clone)]
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

impl Display for Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", type_name::<Self>())
    }
}
