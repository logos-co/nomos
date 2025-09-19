use serde::de::{Error, SeqAccess, Visitor};

use crate::mantle::ops::{Op, opcode};

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

impl<'de> Visitor<'de> for OpWireVisitor {
    type Value = Op;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a binary encoded Op")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let opcode: u8 = seq
            .next_element()?
            .ok_or_else(|| Error::custom("missing opcode"))?;

        match opcode {
            opcode::INSCRIBE => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing Inscription payload"))?;
                Ok(Op::ChannelInscribe(payload))
            }
            opcode::BLOB => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing Blob payload"))?;
                Ok(Op::ChannelBlob(payload))
            }
            opcode::SET_CHANNEL_KEYS => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing ChannelSetKeys payload"))?;
                Ok(Op::ChannelSetKeys(payload))
            }
            opcode::NATIVE => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing Native payload"))?;
                Ok(Op::Native(payload))
            }
            opcode::SDP_DECLARE => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing SDPDeclare payload"))?;
                Ok(Op::SDPDeclare(payload))
            }
            opcode::SDP_WITHDRAW => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing SDPWithdraw payload"))?;
                Ok(Op::SDPWithdraw(payload))
            }
            opcode::SDP_ACTIVE => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing SDPActive payload"))?;
                Ok(Op::SDPActive(payload))
            }
            opcode::LEADER_CLAIM => {
                let payload = seq
                    .next_element()?
                    .ok_or_else(|| Error::custom("missing LeaderClaim payload"))?;
                Ok(Op::LeaderClaim(payload))
            }
            _ => Err(Error::custom(format!("unknown opcode: {opcode}"))),
        }
    }
}
