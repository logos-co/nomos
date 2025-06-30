use serde::{Deserialize, Serialize};

use super::{
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
    serde_, Op,
};

/// Core set of supported Mantle operations and their serialization behaviour.
#[derive(Serialize)]
#[serde(untagged)]
pub enum OpSer<'a> {
    Inscribe(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{INSCRIBE}, InscriptionOp, _>")]
        &'a InscriptionOp,
    ),
    Blob(#[serde(serialize_with = "serde_::serialize_op_variant::<{BLOB}, BlobOp, _>")] &'a BlobOp),
    SetChannelKeys(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{SET_CHANNEL_KEYS}, SetChannelKeysOp, _>"
        )]
        &'a SetChannelKeysOp,
    ),
    Native(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{NATIVE}, NativeOp, _>")]
        &'a NativeOp,
    ),
    SDPDeclare(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{SDP_DECLARE}, SDPDeclareOp, _>")]
        &'a SDPDeclareOp,
    ),
    SDPWithdraw(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{SDP_WITHDRAW}, SDPWithdrawOp, _>"
        )]
        &'a SDPWithdrawOp,
    ),
    SDPActive(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{SDP_ACTIVE}, SDPActiveOp, _>")]
        &'a SDPActiveOp,
    ),
    LeaderClaim(
        #[serde(
            serialize_with = "serde_::serialize_op_variant::<{LEADER_CLAIM}, LeaderClaimOp, _>"
        )]
        &'a LeaderClaimOp,
    ),
}

impl<'a> From<&'a Op> for OpSer<'a> {
    fn from(value: &'a Op) -> Self {
        match value {
            Op::Inscribe(op) => OpSer::Inscribe(op),
            Op::Blob(op) => OpSer::Blob(op),
            Op::SetChannelKeys(op) => OpSer::SetChannelKeys(op),
            Op::Native(op) => OpSer::Native(op),
            Op::SDPDeclare(op) => OpSer::SDPDeclare(op),
            Op::SDPWithdraw(op) => OpSer::SDPWithdraw(op),
            Op::SDPActive(op) => OpSer::SDPActive(op),
            Op::LeaderClaim(op) => OpSer::LeaderClaim(op),
        }
    }
}

/// Core set of supported Mantle operations and their deserialization behaviour.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum OpDe {
    Inscribe(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{INSCRIBE}, InscriptionOp, _>"
        )]
        InscriptionOp,
    ),
    Blob(#[serde(deserialize_with = "serde_::deserialize_op_variant::<{BLOB}, BlobOp, _>")] BlobOp),
    SetChannelKeys(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SET_CHANNEL_KEYS}, SetChannelKeysOp, _>"
        )]
        SetChannelKeysOp,
    ),
    Native(
        #[serde(deserialize_with = "serde_::deserialize_op_variant::<{NATIVE}, NativeOp, _>")]
        NativeOp,
    ),
    SDPDeclare(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_DECLARE}, SDPDeclareOp, _>"
        )]
        SDPDeclareOp,
    ),
    SDPWithdraw(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_WITHDRAW}, SDPWithdrawOp, _>"
        )]
        SDPWithdrawOp,
    ),
    SDPActive(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{SDP_ACTIVE}, SDPActiveOp, _>"
        )]
        SDPActiveOp,
    ),
    LeaderClaim(
        #[serde(
            deserialize_with = "serde_::deserialize_op_variant::<{LEADER_CLAIM}, LeaderClaimOp, _>"
        )]
        LeaderClaimOp,
    ),
}

impl From<OpDe> for Op {
    fn from(value: OpDe) -> Self {
        match value {
            OpDe::Inscribe(inscribe) => Self::Inscribe(inscribe),
            OpDe::Blob(blob) => Self::Blob(blob),
            OpDe::SetChannelKeys(set_channel_keys) => Self::SetChannelKeys(set_channel_keys),
            OpDe::Native(native) => Self::Native(native),
            OpDe::SDPDeclare(sdp_declare) => Self::SDPDeclare(sdp_declare),
            OpDe::SDPWithdraw(sdp_withdraw) => Self::SDPWithdraw(sdp_withdraw),
            OpDe::SDPActive(sdp_active) => Self::SDPActive(sdp_active),
            OpDe::LeaderClaim(leader_claim) => Self::LeaderClaim(leader_claim),
        }
    }
}
