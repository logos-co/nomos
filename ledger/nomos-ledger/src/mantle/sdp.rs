use nomos_core::{
    block::BlockNumber,
    mantle::ops::sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
    sdp::{
        state::{DeclarationStateError, TransientDeclarationState},
        DeclarationId, DeclarationState, Nonce, ServiceParameters,
    },
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Error;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SdpLedgerError {
    #[error("Invalid Sdp state transition: {0:?}")]
    SdpStateError(#[from] DeclarationStateError),
    #[error("Sdp declaration id not found: {0:?}")]
    SdpDeclarationNotFound(DeclarationId),
    #[error("Invalid sdp message nonce: {0:?}")]
    SdpInvalidNonce(Nonce),
    #[error("Duplicate sdp declaration id: {0:?}")]
    SdpDuplicateDeclaration(DeclarationId),
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpLedger {
    pub declarations: rpds::HashTrieMapSync<DeclarationId, DeclarationState>,
}

impl Default for SdpLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl SdpLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            declarations: rpds::HashTrieMapSync::new_sync(),
        }
    }

    pub fn apply_declare_msg(
        mut self,
        block_number: BlockNumber,
        op: &SDPDeclareOp,
    ) -> Result<Self, Error> {
        let declaration_id = op.declaration_id();
        if self.declarations.contains_key(&declaration_id) {
            return Err(SdpLedgerError::SdpDuplicateDeclaration(declaration_id).into());
        }

        self.declarations = self.declarations.insert(
            declaration_id,
            DeclarationState::new(block_number, op.service_type, op.locked_note_id, op.zk_id),
        );

        Ok(self)
    }

    pub fn apply_active_msg(
        mut self,
        block_number: BlockNumber,
        service_params: &ServiceParameters,
        op: &SDPActiveOp,
    ) -> Result<Self, Error> {
        let Some(current_state) = self.declarations.get_mut(&op.declaration_id) else {
            return Err(SdpLedgerError::SdpDeclarationNotFound(op.declaration_id).into());
        };

        if op.nonce <= current_state.nonce {
            return Err(SdpLedgerError::SdpInvalidNonce(op.nonce).into());
        }

        TransientDeclarationState::try_from_state(block_number, current_state, service_params)?
            .try_into_active(block_number)?;

        current_state.nonce = op.nonce;

        Ok(self)
    }

    pub fn apply_withdrawn_msg(
        mut self,
        block_number: BlockNumber,
        service_params: &ServiceParameters,
        op: &SDPWithdrawOp,
    ) -> Result<Self, Error> {
        let Some(current_state) = self.declarations.get_mut(&op.declaration_id) else {
            return Err(SdpLedgerError::SdpDeclarationNotFound(op.declaration_id).into());
        };

        if op.nonce <= current_state.nonce {
            return Err(SdpLedgerError::SdpInvalidNonce(op.nonce).into());
        }

        TransientDeclarationState::try_from_state(block_number, current_state, service_params)?
            .try_into_withdrawn(block_number, service_params)?;

        current_state.nonce = op.nonce;

        Ok(self)
    }

    pub fn get_declaration(&self, id: &DeclarationId) -> Result<&DeclarationState, Error> {
        self.declarations
            .get(id)
            .ok_or_else(|| SdpLedgerError::SdpDeclarationNotFound(*id).into())
    }
}
