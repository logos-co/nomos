pub mod blob;
pub mod channel_keys;
pub mod inscribe;
pub(crate) mod internal;
mod leader_claim;
pub mod native;
pub mod opcode;
pub mod sdp;
mod serde_;
mod wire;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{
    gas::{Gas, GasConstants},
    ops::{
        blob::{BlobOp, DA_COLUMNS, DA_ELEMENT_SIZE},
        channel_keys::SetChannelKeysOp,
        inscribe::InscriptionOp,
        leader_claim::LeaderClaimOp,
        native::NativeOp,
        opcode::{
            BLOB, INSCRIBE, LEADER_CLAIM, NATIVE, SDP_ACTIVE, SDP_DECLARE, SDP_WITHDRAW,
            SET_CHANNEL_KEYS,
        },
        sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
    },
};
use crate::mantle::ops::{
    internal::{OpDe, OpSer},
    wire::OpWireVisitor,
};
pub type Ed25519PublicKey = [u8; 32];
pub type ChannelId = u64;

/// Core set of supported Mantle operations.
///
/// This type serves as the public-facing representation of [`OpSer`] and
/// [`OpDe`], delegating default serialization and deserialization to them.
///
/// Serialization and deserialization are performed using [`serde_::WireOpSer`]
/// and [`serde_::WireOpDe`], which introduce a custom `opcode` tag to identify
/// the correct variant. Due to limitations in [`bincode`] and [`serde`]'s
/// `#[serde(untagged)]` enums, binary deserialization is routed through
/// [`OpWireVisitor`], which correctly handles `opcode` to select the
/// appropriate variant.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Op {
    Inscribe(InscriptionOp),
    Blob(BlobOp),
    SetChannelKeys(SetChannelKeysOp),
    Native(NativeOp),
    SDPDeclare(SDPDeclareOp),
    SDPWithdraw(SDPWithdrawOp),
    SDPActive(SDPActiveOp),
    LeaderClaim(LeaderClaimOp),
}

/// Delegates serialization through the [`OpInternal`] representation.
impl Serialize for Op {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let op_ser = OpSer::from(self);
        op_ser.serialize(serializer)
    }
}

/// Delegates deserialization through the [`OpInternal`] representation.
///
/// If the deserializer is non-human-readable, it assumes the input was encoded
/// using [`wire`] and uses [`OpWireVisitor`] to deserialize it.
/// Otherwise, it falls back to deserializing via [`OpInternal`]'s default
/// behaviour.
///
/// # Notes
/// - When using the `wire` format, the tuple must contain the exact number of
///   fields expected by [`WireOpDes`](serde_::WireOpDes), or unexpected
///   behaviour may occur.
impl<'de> Deserialize<'de> for Op {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            OpDe::deserialize(deserializer).map(Self::from)
        } else {
            deserializer.deserialize_tuple(2, OpWireVisitor)
        }
    }
}

impl Op {
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        match self {
            Self::Inscribe(_) => INSCRIBE,
            Self::Blob(_) => BLOB,
            Self::SetChannelKeys(_) => SET_CHANNEL_KEYS,
            Self::Native(_) => NATIVE,
            Self::SDPDeclare(_) => SDP_DECLARE,
            Self::SDPWithdraw(_) => SDP_WITHDRAW,
            Self::SDPActive(_) => SDP_ACTIVE,
            Self::LeaderClaim(_) => LEADER_CLAIM,
        }
    }

    #[must_use]
    pub fn as_sign_bytes(&self) -> bytes::Bytes {
        let mut buff = bytes::BytesMut::new();
        buff.extend_from_slice(&[self.opcode()]);
        // TODO: add ops payload
        if let Self::Blob(blob_op) = self {
            buff.extend_from_slice(&blob_op.as_sign_bytes());
        }
        buff.freeze()
    }

    #[must_use]
    pub fn execution_gas<Constants: GasConstants>(&self) -> Gas {
        match self {
            Self::Inscribe(_) => Constants::CHANNEL_INSCRIBE,
            Self::Blob(BlobOp { blob_size, .. }) => {
                let sample_size = blob_size / (DA_COLUMNS * DA_ELEMENT_SIZE);
                Constants::CHANNEL_BLOB_BASE + Constants::CHANNEL_BLOB_SIZED * sample_size
            }
            Self::SetChannelKeys(_) => Constants::CHANNEL_SET_KEYS,
            Self::Native(_) => Gas::MAX,
            Self::SDPDeclare(_) => Constants::SDP_DECLARE,
            Self::SDPWithdraw(_) => Constants::SDP_WITHDRAW,
            Self::SDPActive(_) => Constants::SDP_ACTIVE,
            Self::LeaderClaim(_) => Constants::LEADER_CLAIM,
        }
    }

    #[must_use]
    pub fn da_gas_cost(&self) -> Gas {
        if let Self::Blob(BlobOp {
            blob_size,
            da_storage_gas_price,
            ..
        }) = self
        {
            da_storage_gas_price * blob_size
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{blob::BlobOp, Op};
    use crate::wire;

    #[test]
    fn test_json_serialize_deserialize_blob_op() {
        let zeros = json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]);
        let payload = json!({"channel": 0, "blob": zeros, "blob_size": 0, "after_tx": Value::Null, "signer": zeros, "da_storage_gas_price": 0});
        let repr = json!({"opcode": 0x01, "payload": payload});
        println!("{:?}", serde_json::to_string(&repr).unwrap());
        let op = Op::Blob(BlobOp {
            channel: 0,
            blob: [0; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            after_tx: None,
            signer: [0; 32],
        });
        let serialized = serde_json::to_value(&op).unwrap();
        assert_eq!(serialized, repr);
        let deserialized = serde_json::from_value::<Op>(repr).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn test_bincode_serialize_deserialize_blob_op() {
        // opcode + payload
        let expected_bincode = vec![
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];

        let blob_op = BlobOp {
            channel: 0,
            blob: [0; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            after_tx: None,
            signer: [0; 32],
        };
        let op = Op::Blob(blob_op);

        let serialized = wire::serialize(&op).unwrap();
        assert_eq!(serialized, expected_bincode);
        let deserialized = wire::deserialize::<Op>(&serialized).unwrap();
        assert_eq!(deserialized, op);
    }
}
