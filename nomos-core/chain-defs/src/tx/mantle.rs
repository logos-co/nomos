// Represents the type of a Mantle Operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    Inscribe = 0x00,
    Blob = 0x01,
    SetChannelKeys = 0x02,
    Native = 0x03,
    SdpDeclare = 0x04,
    SdpWithdraw = 0x05,
    SdpActive = 0x06,
    LeaderClaim = 0x07,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZkSignaturePublicKey(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub value: u64,
    pub public_key: ZkSignaturePublicKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteId(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerTx {
    pub inputs: Vec<NoteId>,
    pub outputs: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleTx {
    pub ops: Vec<Op>,
    pub ledger_tx: LedgerTx,
    pub gas_price: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpProof(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerTxProof(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedMantleTx {
    pub tx: MantleTx,
    /// A vector where each element corresponds to an operation in `tx.ops`.
    /// `Some(OpProof)` indicates a proof is present for that operation, `None`
    /// if not required.
    pub op_proofs: Vec<Option<OpProof>>,
    pub ledger_tx_proof: LedgerTxProof,
}
