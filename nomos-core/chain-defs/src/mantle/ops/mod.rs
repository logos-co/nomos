pub mod channel;
pub(crate) mod internal;
pub mod leader_claim;
pub mod native;
pub mod opcode;
pub mod sdp;
mod serde_;
mod wire;

use bytes::Bytes;
use channel::{
    blob::{BlobOp, DA_COLUMNS, DA_ELEMENT_SIZE},
    inscribe::InscriptionOp,
    set_keys::SetKeysOp,
};
use groth16::Fr;
use num_bigint::BigUint;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{
    gas::{Gas, GasConstants},
    ops::{
        leader_claim::LeaderClaimOp,
        native::NativeOp,
        opcode::{
            BLOB, INSCRIBE, LEADER_CLAIM, NATIVE, SDP_ACTIVE, SDP_DECLARE, SDP_WITHDRAW,
            SET_CHANNEL_KEYS,
        },
        sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
    },
};
use crate::{
    mantle::ops::{
        internal::{OpDe, OpSer},
        wire::OpWireVisitor,
    },
    proofs::zksig,
};

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
    ChannelInscribe(InscriptionOp),
    ChannelBlob(BlobOp),
    ChannelSetKeys(SetKeysOp),
    Native(NativeOp),
    SDPDeclare(SDPDeclareOp),
    SDPWithdraw(SDPWithdrawOp),
    SDPActive(SDPActiveOp),
    LeaderClaim(LeaderClaimOp),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpProof {
    Ed25519Sig(ed25519::Signature),
    ZkSig(zksig::DummyZkSignature),
    ZkAndEd25519Sigs {
        zk_sig: zksig::DummyZkSignature,
        ed25519_sig: ed25519::Signature,
    },
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
            Self::ChannelInscribe(_) => INSCRIBE,
            Self::ChannelBlob(_) => BLOB,
            Self::ChannelSetKeys(_) => SET_CHANNEL_KEYS,
            Self::Native(_) => NATIVE,
            Self::SDPDeclare(_) => SDP_DECLARE,
            Self::SDPWithdraw(_) => SDP_WITHDRAW,
            Self::SDPActive(_) => SDP_ACTIVE,
            Self::LeaderClaim(_) => LEADER_CLAIM,
        }
    }

    #[must_use]
    pub fn payload_bytes(&self) -> Bytes {
        match self {
            Self::ChannelInscribe(channel_inscribre) => channel_inscribre.payload_bytes(),
            Self::ChannelBlob(channel_blob) => channel_blob.payload_bytes(),
            Self::ChannelSetKeys(channel_keys) => channel_keys.payload_bytes(),
            Self::Native(native) => native.payload_bytes(),
            Self::SDPDeclare(sdp_declare) => sdp_declare.payload_bytes(),
            Self::SDPWithdraw(sdp_withdraw) => sdp_withdraw.payload_bytes(),
            Self::SDPActive(sdp_active) => sdp_active.payload_bytes(),
            Self::LeaderClaim(leader_claim) => leader_claim.payload_bytes(),
        }
    }

    #[must_use]
    pub fn as_signing_fr(&self) -> Vec<Fr> {
        let mut buff = Vec::new();
        buff.push(BigUint::from(self.opcode()).into());

        for chunk in self.payload_bytes().chunks(31) {
            buff.push(BigUint::from_bytes_le(chunk).into());
        }
        buff
    }

    #[must_use]
    pub fn execution_gas<Constants: GasConstants>(&self) -> Gas {
        match self {
            Self::ChannelInscribe(_) => Constants::CHANNEL_INSCRIBE,
            Self::ChannelBlob(BlobOp { blob_size, .. }) => {
                let sample_size = blob_size / (DA_COLUMNS * DA_ELEMENT_SIZE);
                Constants::CHANNEL_BLOB_BASE + Constants::CHANNEL_BLOB_SIZED * sample_size
            }
            Self::ChannelSetKeys(_) => Constants::CHANNEL_SET_KEYS,
            Self::Native(_) => Gas::MAX,
            Self::SDPDeclare(_) => Constants::SDP_DECLARE,
            Self::SDPWithdraw(_) => Constants::SDP_WITHDRAW,
            Self::SDPActive(_) => Constants::SDP_ACTIVE,
            Self::LeaderClaim(_) => Constants::LEADER_CLAIM,
        }
    }

    #[must_use]
    pub fn da_gas_cost(&self) -> Gas {
        if let Self::ChannelBlob(BlobOp {
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
    use std::sync::LazyLock;

    use serde_json::json;

    use super::{channel::blob::BlobOp, Op};
    use crate::codec;

    // nothing special, just some valid bytes
    static VK: LazyLock<ed25519_dalek::VerifyingKey> = LazyLock::new(|| {
        ed25519_dalek::VerifyingKey::from_bytes(&[
            215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58, 14, 225, 114,
            243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26,
        ])
        .unwrap()
    });

    #[test]
    fn test_json_serialize_deserialize_blob_op() {
        let zeros = json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]);
        let zero_string = "0000000000000000000000000000000000000000000000000000000000000000";
        let payload = json!({"channel": zero_string, "blob": zeros, "blob_size": 0, "parent": zero_string, "signer": *VK, "da_storage_gas_price": 0});
        let repr = json!({"opcode": 0x01, "payload": payload});
        println!("{:?}", serde_json::to_string(&repr).unwrap());
        let op = Op::ChannelBlob(BlobOp {
            channel: [0; 32].into(),
            blob: [0; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            parent: [0; 32].into(),
            signer: *VK,
        });
        let serialized = serde_json::to_value(&op).unwrap();
        assert_eq!(serialized, repr);
        let deserialized = serde_json::from_value::<Op>(repr).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn test_bincode_serialize_deserialize_blob_op() {
        // opcode + payload
        // TODO: use more efficient repr for the ed25519 key
        let expected_bincode = vec![
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 0, 0,
            0, 0, 0, 0, 0, 215, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7, 58,
            14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81, 26,
        ];

        let blob_op = BlobOp {
            channel: [0; 32].into(),
            blob: [0; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            parent: [0; 32].into(),
            signer: *VK,
        };
        let op = Op::ChannelBlob(blob_op);

        let serialized =
            <Op as codec::SerdeOp>::serialize(&op).expect("Op should be able to be serialized");
        assert_eq!(serialized.to_vec(), expected_bincode);
        let deserialized: Op = <Op as codec::SerdeOp>::deserialize(&serialized).unwrap();
        assert_eq!(deserialized, op);
    }
}
