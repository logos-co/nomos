use kzgrs_backend::dispersal::BlobInfo;
use nomos_sdp_core::SdpMessage;

use super::{MantleOp, MantleTx, Opcode, SignedMantleTx};

#[derive(Debug, Clone)]
pub struct InscribeOp;

#[derive(Debug, Clone)]
pub struct SetChannelKeysOp;

#[derive(Debug, Clone)]
pub struct NativeOp;

#[derive(Debug, Clone)]
pub struct LeaderClaimOp;

#[derive(Debug, Clone)]
pub enum Op<SdpMetadata> {
    Inscribe(InscribeOp),
    Blob(BlobInfo),
    SetChannelKeys(SetChannelKeysOp),
    Native(NativeOp),
    Sdp(SdpMessage<SdpMetadata>),
    LeaderClaim(LeaderClaimOp),
}

impl<SdpMetadata> MantleOp for Op<SdpMetadata> {
    fn get_opcode(&self) -> Opcode {
        match self {
            Self::Inscribe(_) => Opcode::Inscribe,
            Self::Blob(_) => Opcode::Blob,
            Self::SetChannelKeys(_) => Opcode::SetChannelKeys,
            Self::Native(_) => Opcode::Native,
            Self::Sdp(op) => match op {
                SdpMessage::Declare(_) => Opcode::SdpDeclare,
                SdpMessage::Activity(_) => Opcode::SdpActive,
                SdpMessage::Withdraw(_) => Opcode::SdpWithdraw,
            },
            Self::LeaderClaim(_) => Opcode::LeaderClaim,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ZkSignaturePublicKey(pub [u8; 32]);

#[derive(Debug, Clone)]
pub struct Note {
    pub value: u64,
    pub public_key: ZkSignaturePublicKey,
}

#[derive(Debug, Clone)]
pub struct NoteId(pub [u8; 32]);

#[derive(Debug, Clone)]
pub struct LedgerTx {
    pub inputs: Vec<NoteId>,
    pub outputs: Vec<Note>,
}

#[derive(Debug, Clone)]
pub struct Tx<SdpMetadata> {
    pub ops: Vec<Op<SdpMetadata>>,
    pub ledger_tx: LedgerTx,
    pub gas_price: u64,
}

impl<SdpMetadata> MantleTx for Tx<SdpMetadata> {
    type LedgerTx = LedgerTx;
    type Op = Op<SdpMetadata>;

    fn get_operations(&self) -> impl Iterator<Item = &Self::Op> {
        self.ops.iter()
    }

    fn get_ledger_tx(&self) -> &Self::LedgerTx {
        &self.ledger_tx
    }

    fn get_gas_price(&self) -> u64 {
        self.gas_price
    }
}

#[derive(Debug, Clone)]
pub struct OpProof(pub Vec<u8>);

#[derive(Debug, Clone)]
pub struct LedgerTxProof(pub Vec<u8>);

#[derive(Debug, Clone)]
pub struct SignedTx<SdpMetadata> {
    pub tx: Tx<SdpMetadata>,
    /// A vector where each element corresponds to an operation in `tx.ops`.
    /// `Some(OpProof)` indicates a proof is present for that operation, `None`
    /// if not required.
    pub op_proofs: Vec<Option<OpProof>>,
    pub ledger_tx_proof: LedgerTxProof,
}

impl<SdpMetadata> SignedMantleTx for SignedTx<SdpMetadata> {
    type Tx = Tx<SdpMetadata>;
    type OpProof = OpProof;
    type LedgerTxProof = LedgerTxProof;

    fn get_mantle_tx(&self) -> &Self::Tx {
        &self.tx
    }

    fn get_op_proofs(&self) -> impl Iterator<Item = &Option<Self::OpProof>> {
        self.op_proofs.iter()
    }

    fn get_ledger_tx_proof(&self) -> &Self::LedgerTxProof {
        &self.ledger_tx_proof
    }
}
