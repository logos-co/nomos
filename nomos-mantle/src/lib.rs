pub mod tx;

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

pub trait MantleOp {
    fn get_opcode(&self) -> Opcode;
}

pub trait MantleTx {
    type LedgerTx;
    type Op;

    fn get_operations(&self) -> impl Iterator<Item = &Self::Op>;
    fn get_ledger_tx(&self) -> &Self::LedgerTx;
    fn get_gas_price(&self) -> u64;
}

pub trait SignedMantleTx {
    type Tx;
    type OpProof;
    type LedgerTxProof;

    fn get_mantle_tx(&self) -> &Self::Tx;
    fn get_op_proofs(&self) -> impl Iterator<Item = &Option<Self::OpProof>>;
    fn get_ledger_tx_proof(&self) -> &Self::LedgerTxProof;
}
