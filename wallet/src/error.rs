use nomos_core::header::HeaderId;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("Requested wallet state for unknown block: {0}")]
    UnknownBlock(HeaderId),
    #[error("Wallet does not have enough funds, available={available}")]
    InsufficientFunds { available: u64 },
}
