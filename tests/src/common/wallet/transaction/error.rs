//! Shared error type for wallet transaction preparation.

// TODO(RFC-480): UTXO test wallet.
// use lb_core::mantle::{NoteId, VerificationError, gas::GasOverflow, transactions::TxBuilderError};
use lb_core::mantle::{
    VerificationError, gas::GasOverflow, notes::NoteNf, transactions::TxBuilderError,
};
use lb_key_management_system_service::keys::{ZkPublicKey, errors::KeyError};
use lb_utils::bounded::BoundedError;
use lb_wallet::WalletError;
// TODO(RFC-480): the ZkSignature scheme is removed (Common Cryptographic
// Components 1.3.0).
// use lb_zksign::ZkSignError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletTransactionError {
    #[error(transparent)]
    Funding(#[from] WalletError),
    // TODO(RFC-480): UTXO test wallet, a note is spent by its nullifier.
    // #[error("missing funding UTXO for transfer input {note_id:?}")]
    // MissingFundingInput { note_id: NoteId },
    // #[error("missing signing key for transfer input {note_id:?}")]
    // MissingSigningKey { note_id: NoteId },
    #[error("missing funding note for transfer input {nullifier:?}")]
    MissingFundingInput { nullifier: NoteNf },
    #[error("missing signing key for the transfer input of {public_key:?}")]
    MissingSigningKey { public_key: ZkPublicKey },
    #[error("leading inscription operation does not match its transaction signing key")]
    InvalidLeadingInscriptionSigner,
    // TODO(RFC-480): the ZkSignature scheme is removed (Common Cryptographic
    // Components 1.3.0).
    // #[error(transparent)]
    // Signing(#[from] ZkSignError),
    #[error(transparent)]
    Proving(#[from] KeyError),
    #[error(transparent)]
    Verification(#[from] VerificationError),
    #[error(transparent)]
    Gas(#[from] GasOverflow),
    #[error(transparent)]
    Builder(#[from] TxBuilderError),
    #[error("wallet transaction output total overflowed u64")]
    OutputTotalOverflow,
    #[error("wallet transaction inputs cannot cover its outputs")]
    FeeAccounting,
    #[error(transparent)]
    BoundedError(#[from] BoundedError),
    #[error("Mantle transaction error: {0}")]
    SignedOpsError(#[from] lb_core::mantle::transactions::tx_list::signed_ops::Error),
}
