pub mod blob;
pub mod channel_keys;
pub mod inscribe;
mod leader_claim;
pub mod native;
pub mod opcode;
pub mod sdp;
mod serde_;

use serde::{Deserialize, Serialize};

use crate::{
    gas::{Gas, GasConstants, GasPrice},
    ops::{
        blob::BlobOp,
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
pub type Ed25519PublicKey = [u8; 32];
pub type ChannelId = u64;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Op {
    Inscribe(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{INSCRIBE}, InscriptionOp, _>")]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{INSCRIBE}, InscriptionOp, _>"
        )]
        InscriptionOp,
    ),
    Blob(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{BLOB}, BlobOp, _>")]
        #[serde(deserialize_with = "serde_::deserialize_op_variant::<{BLOB}, BlobOp, _>")]
        BlobOp,
    ),
    SetChannelKeys(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{SET_CHANNEL_KEYS}, SetChannelKeysOp, _>"
        )]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SET_CHANNEL_KEYS}, SetChannelKeysOp, _>"
        )]
        SetChannelKeysOp,
    ),
    Native(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{NATIVE}, NativeOp, _>")]
        #[serde(deserialize_with = "serde_::deserialize_op_variant::<{NATIVE}, NativeOp, _>")]
        NativeOp,
    ),
    SDPDeclare(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{SDP_DECLARE}, SDPDeclareOp, _>")]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_DECLARE}, SDPDeclareOp, _>"
        )]
        SDPDeclareOp,
    ),
    SDPWithdraw(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{SDP_WITHDRAW}, SDPWithdrawOp, _>"
        )]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_WITHDRAW}, SDPWithdrawOp, _>"
        )]
        SDPWithdrawOp,
    ),
    SDPActive(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{SDP_ACTIVE}, SDPActiveOp, _>")]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_ACTIVE}, SDPActiveOp, _>"
        )]
        SDPActiveOp,
    ),
    LeaderClaim(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{LEADER_CLAIM}, LeaderClaimOp, _>"
        )]
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{LEADER_CLAIM}, LeaderClaimOp, _>"
        )]
        LeaderClaimOp,
    ),
}

impl GasPrice for Op {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        match self {
            Self::Inscribe(op) => op.gas_price::<Constants>(),
            Self::Blob(op) => op.gas_price::<Constants>(),
            Self::SetChannelKeys(op) => op.gas_price::<Constants>(),
            Self::Native(op) => op.gas_price::<Constants>(),
            Self::SDPDeclare(op) => op.gas_price::<Constants>(),
            Self::SDPWithdraw(op) => op.gas_price::<Constants>(),
            Self::SDPActive(op) => op.gas_price::<Constants>(),
            Self::LeaderClaim(op) => op.gas_price::<Constants>(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::ops::{Op, blob::BlobOp};

    #[test]
    fn test_serialize_deserialize_blob_op() {
        let zeros = json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]);
        let payload = json!({"channel": 0, "blob": zeros, "blob_size": 0, "after_tx": Value::Null, "signer": zeros});
        let repr = json!({"opcode": 0x01, "payload": payload});
        println!("{:?}", serde_json::to_string(&repr).unwrap());
        let op = Op::Blob(BlobOp {
            channel: 0,
            blob: [0; 32],
            blob_size: 0,
            after_tx: None,
            signer: [0; 32],
        });
        let serialized = serde_json::to_value(&op).unwrap();
        assert_eq!(serialized, repr);
        let deserialized = serde_json::from_value::<Op>(repr).unwrap();
        assert_eq!(deserialized, op);
    }
}
