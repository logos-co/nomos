use thiserror::Error;

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("Requested wallet state for unknown block")]
    UnknownBlock,
}
