use serde::de::{Error, Visitor};

use crate::mantle::{encoding::decode_op, ops::Op};

/// Visitor for deserializing binary-encoded Mantle operations using the
/// [`wire`](crate::codec) format. Although [`Op`] and [`OpInternal`] are
/// untagged enums in Serde terms, binary serde is handled through
/// custom logic using [`WireOpSer`](crate::mantle::ops::serde_::WireOpSer) and
/// [`WireOpDes`](crate::mantle::ops::serde_::WireOpDes). They add an
/// explicit `opcode` tag to identify the variant.
///
/// This visitor is responsible for interpreting that tagged binary format and
/// deserializing the appropriate variant accordingly.
pub struct OpWireVisitor;

impl Visitor<'_> for OpWireVisitor {
    type Value = Op;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a binary encoded Op")
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: Error,
    {
        decode_op(v).map(|(_, op)| op).map_err(E::custom)
    }
}
