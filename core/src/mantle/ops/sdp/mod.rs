pub mod active;
pub mod declare;
pub mod withdraw;

pub use active::{SDPActiveExecutionContext, SDPActiveValidationContext};
pub use declare::{SDPDeclareExecutionContext, SDPDeclareVerificationContext};
use lb_cryptarchia_engine::Epoch;
use lb_key_management_system_keys::keys::ZkPublicKey;
use lb_zktransfer::ZkTransferInputsError;
use thiserror::Error;
pub use withdraw::{SDPWithdrawExecutionContext, SDPWithdrawValidationContext};

// TODO(RFC-480): SDP Operations spend nullifiers, not note ids; `NoteId` left the
// `mantle` imports.
use crate::{
    mantle::{Value, note_set::NoteSetError},
    sdp::{DeclarationId, Nonce, ProviderId, ServiceType, StakeThreshold},
};

pub type SDPDeclareOp = crate::sdp::DeclarationMessage;
pub type SDPWithdrawOp = crate::sdp::WithdrawMessage;
pub type SDPActiveOp = crate::sdp::ActiveMessage;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SdpError {
    // TODO(RFC-480): SDP Operations spend nullifiers, not note ids.
    // #[error("Note: {0:?} isn't in the ledger")]
    // InexistingNote(NoteId),
    // #[error("Note {0:?} is a channel note and cannot be used as service collateral")]
    // ChannelNote(NoteId),
    // #[error("Invalid SDP declare ZkSignature")]
    // InvalidZkSignature,
    #[error("Invalid SDP declare EDDSA signature")]
    InvalidEddsaSignature,
    #[error("Duplicate sdp declaration id: {0:?}")]
    DuplicateDeclaration(DeclarationId),
    #[error("Duplicate provider_id within service {service_type:?}: {provider_id:?}")]
    DuplicateProviderId {
        service_type: ServiceType,
        provider_id: Box<ProviderId>,
    },
    #[error("Duplicate zk_id within service {service_type:?}: {zk_id:?}")]
    DuplicateZkId {
        service_type: ServiceType,
        zk_id: ZkPublicKey,
    },
    // TODO(RFC-480): SDP Operations spend nullifiers, not note ids.
    // #[error("Note {note_id:?} insufficient value: {value}")]
    // NoteInsufficientValue { note_id: NoteId, value: u64 },
    // #[error("Note {note_id:?} already used for service {service_type:?}")]
    // NoteAlreadyUsedForService {
    //     note_id: NoteId,
    //     service_type: ServiceType,
    // },
    // #[error(
    //     "An unexpected error occurred during sdp declare execution, please validate the op before executing"
    // )]
    // UnexpectedError,
    #[error("Sdp declaration id could not be found: {0:?}")]
    DeclarationNotFound(DeclarationId),
    #[error("Service type could not be found: {0:?}")]
    ServiceNotFound(ServiceType),
    #[error(
        "Sdp declaration has been already scheduled to be withdrawn: {declaration_id:?} at epoch {withdraw_at:?}"
    )]
    DeclarationWithdrawn {
        declaration_id: DeclarationId,
        withdraw_at: Epoch,
    },
    #[error(
        "Invalid sdp message nonce: message_nonce={message_nonce:?}, declaration_nonce={declaration_nonce:?}"
    )]
    InvalidNonce {
        message_nonce: Nonce,
        declaration_nonce: Nonce,
    },
    // TODO(RFC-480): SDP Operations spend nullifiers, not note ids.
    // #[error("Note is not a service note: {0:?}")]
    // NotAServiceNote(NoteId),
    // #[error("Note {note_id:?} not used for {service_type:?}")]
    // NoteNotUsedForService {
    //     note_id: NoteId,
    //     service_type: ServiceType,
    // },
    // #[error("Note {note_id:?} is not corresponding to the one in the declaration {expected:?}")]
    // InvalidServiceNote { note_id: NoteId, expected: NoteId },
    #[error(transparent)]
    NoteSet(#[from] NoteSetError),
    #[error(transparent)]
    ZkTransferInputs(#[from] ZkTransferInputsError),
    #[error("Declared amount {amount} is below the minimum stake {threshold}")]
    AmountBelowMinStake {
        amount: Value,
        threshold: StakeThreshold,
    },
    #[error("The ZkTransfer of the declaration does not verify")]
    InvalidZkTransfer,
}
