use lb_binary_codec::canonical::{BinaryDecode, BinaryEncode, DecodeError};
use lb_key_management_system_keys::keys::Ed25519Signature;
use serde::{Deserialize, Serialize};

use crate::{
    mantle::{
        Op,
        ledger::ProvableOperation,
        ops::{
            ChannelInscribeProof, NoOpProof, OpProofRef, ZkAndEd25519Proof, ZkTransferProof,
            channel::inscribe::Step,
        },
    },
    proofs::channel_multi_sig_proof::ChannelMultiSigProof,
    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the
    // leader reward is a note in the block.
    // proofs::leader_claim_proof::Groth16LeaderClaimProof,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpProof {
    Ed25519Sig(Ed25519Signature),
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // ZkSig(ZkSignature),
    ZkTransfer(ZkTransferProof),
    ZkAndEd25519Sigs(ZkAndEd25519Proof),
    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the
    // leader reward is a note in the block.
    // PoC(Groth16LeaderClaimProof),
    ChannelMultiSigProof(ChannelMultiSigProof),
    None(NoOpProof),
    ChannelInscribe(ChannelInscribeProof),
}

impl OpProof {
    #[must_use]
    pub fn by_ref(&self) -> OpProofRef<'_> {
        self.into()
    }

    /// Helper to decode an `OpProof` for a given `Op`
    pub fn decode_for_op<'i>(input: &'i [u8], op: &Op) -> Result<(&'i [u8], Self), DecodeError> {
        match op {
            Op::ChannelInscribe(op) => ChannelInscribeProof::decode(
                input,
                &op.steps.iter().map(Step::proof_shape).collect(),
            )
            .map(|(rest, proof)| (rest, proof.into())),
            Op::ChannelConfig(op) => decode_proof_for(op, input),
            Op::ChannelDeposit(op) => decode_proof_for(op, input),
            Op::ChannelWithdraw(op) => decode_proof_for(op, input),
            // TODO(RFC-480): CHANNEL_TRANSFER is removed, channel notes move by
            // the steps of CHANNEL_INSCRIBE.
            // Op::ChannelTransfer(op) => decode_proof_for(op, input),
            Op::SDPDeclare(op) => decode_proof_for(op, input),
            Op::SDPWithdraw(op) => decode_proof_for(op, input),
            Op::SDPActive(op) => decode_proof_for(op, input),
            // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are
            // retired; the leader reward is a note in the block.
            // Op::LeaderClaim(op) => decode_proof_for(op, input),
            Op::Transfer(op) => decode_proof_for(op, input),
            Op::ClaimPowReward(op) => decode_proof_for(op, input),
            Op::ProgramRegister(op) => decode_proof_for(op, input),
        }
    }
}

impl BinaryEncode for OpProof {
    fn encoded_length(&self) -> usize {
        self.by_ref().encoded_length()
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        self.by_ref().encode_into(out);
    }
}

fn decode_proof_for<'a, T>(_op: &T, input: &'a [u8]) -> Result<(&'a [u8], OpProof), DecodeError>
where
    T: ProvableOperation,
    T::Proof: BinaryDecode<Context = ()> + Into<OpProof>,
{
    T::Proof::decode(input, &()).map(|(rest, proof)| (rest, proof.into()))
}

pub struct OpProofDecodeContext<'op> {
    pub op: &'op Op,
}

macro_rules! impl_from_proof {
    ($($proof:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$proof> for crate::mantle::ops::OpProof {
                fn from(proof: $proof) -> Self {
                    crate::mantle::ops::OpProof::$variant(proof)
                }
            }
        )*
    };
}

impl_from_proof! {
    Ed25519Signature => Ed25519Sig,
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // ZkSignature => ZkSig,
    ZkTransferProof => ZkTransfer,
    ZkAndEd25519Proof => ZkAndEd25519Sigs,
    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the
    // leader reward is a note in the block.
    // Groth16LeaderClaimProof => PoC,
    ChannelMultiSigProof => ChannelMultiSigProof,
    NoOpProof => None,
    ChannelInscribeProof => ChannelInscribe,
}

macro_rules! impl_try_from_op_proof_for_proof {
    ($($variant:ident => $proof:ty),* $(,)?) => {
        $(
            impl TryFrom<crate::mantle::ops::OpProof> for $proof {
                type Error = crate::mantle::ops::OpProof;

                fn try_from(op_proof: crate::mantle::ops::OpProof) -> Result<Self, Self::Error> {
                    match op_proof {
                        crate::mantle::ops::OpProof::$variant(inner) => Ok(inner),
                        other => Err(other),
                    }
                }
            }
        )*
    };
}

impl_try_from_op_proof_for_proof! {
    Ed25519Sig => Ed25519Signature,
    // TODO(RFC-480): a ZkTransfer replaces the ZkSignature of Mantle Operations.
    // ZkSig => ZkSignature,
    ZkTransfer => ZkTransferProof,
    ZkAndEd25519Sigs => ZkAndEd25519Proof,
    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired; the
    // leader reward is a note in the block.
    // PoC => Groth16LeaderClaimProof,
    ChannelMultiSigProof => ChannelMultiSigProof,
    None => NoOpProof,
    ChannelInscribe => ChannelInscribeProof,
}

#[cfg(any(test, feature = "test-utils"))]
pub mod samples {
    // TODO(RFC-480): no Operation is proven by a ZkSignature.
    // use lb_groth16::{COMPRESSED_PROOF_SIZE, CompressedGroth16Proof};
    // use lb_key_management_system_keys::keys::{Ed25519Signature, ZkSignature};
    use lb_key_management_system_keys::keys::Ed25519Signature;

    use crate::mantle::{OpProof, ledger::ProvableOperation};

    pub trait SampleProof {
        fn sample() -> Self;
    }

    impl SampleProof for Ed25519Signature {
        fn sample() -> Self {
            Self::zero()
        }
    }

    // TODO(RFC-480): no Operation is proven by a ZkSignature.
    // impl SampleProof for ZkSignature {
    //     fn sample() -> Self {
    //         Self::new(CompressedGroth16Proof::from_bytes(
    //             &[0u8; COMPRESSED_PROOF_SIZE],
    //         ))
    //     }
    // }

    pub fn sample_proof_for<T>(_op: &T) -> OpProof
    where
        T: ProvableOperation,
        T::Proof: SampleProof + Into<OpProof>,
    {
        T::Proof::sample().into()
    }
}
