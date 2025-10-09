use std::collections::{HashMap, HashSet};

use nomos_core::{
    block::BlockNumber,
    mantle::ops::sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
    sdp::{
        DeclarationId, DeclarationState, FinalizedDeclarationState, Nonce, ServiceParameters,
        ServiceType, Session,
        state::{DeclarationStateError, TransientDeclarationState},
    },
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator as _;

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
    #[error("Active session for service {0:?} not found")]
    ActiveSessionNotFound(ServiceType),
    #[error("Forming session for service {0:?} not found")]
    FormingSessionNotFound(ServiceType),
    #[error("Session parameters for {0:?} not found")]
    SessionParamsNotFound(ServiceType),
    #[error("Service parameters are missing for {0:?}")]
    ServiceParamsNotFound(ServiceType),
    #[error("Can't update genesis state during different block number")]
    NotGenesisBlock,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpLedger {
    pub declarations: rpds::HashTrieMapSync<DeclarationId, DeclarationState>,
    active_sessions: rpds::HashTrieMapSync<ServiceType, Session>,
    forming_sessions: rpds::HashTrieMapSync<ServiceType, Session>,
}

impl Default for SdpLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl SdpLedger {
    #[must_use]
    pub fn new() -> Self {
        let mut active_sessions = rpds::HashTrieMapSync::new_sync();
        let mut forming_sessions = rpds::HashTrieMapSync::new_sync();

        for service_type in ServiceType::iter() {
            let initial_active_session = Session {
                session_number: 0,
                declarations: HashSet::new(),
            };
            let initial_forming_session = Session {
                session_number: 1,
                declarations: HashSet::new(),
            };

            active_sessions = active_sessions.insert(service_type, initial_active_session);
            forming_sessions = forming_sessions.insert(service_type, initial_forming_session);
        }

        Self {
            declarations: rpds::HashTrieMapSync::new_sync(),
            active_sessions,
            forming_sessions,
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

        TransientDeclarationState::try_from_state(
            block_number,
            &mut *current_state,
            service_params,
        )?
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

        TransientDeclarationState::try_from_state(
            block_number,
            &mut *current_state,
            service_params,
        )?
        .try_into_withdrawn(block_number, service_params)?;

        current_state.nonce = op.nonce;

        Ok(self)
    }

    pub fn get_declaration(&self, id: &DeclarationId) -> Result<&DeclarationState, Error> {
        self.declarations
            .get(id)
            .ok_or_else(|| SdpLedgerError::SdpDeclarationNotFound(*id).into())
    }

    pub fn try_update_session(
        mut self,
        block_number: BlockNumber,
        service_params: &HashMap<ServiceType, ServiceParameters>,
    ) -> Result<Self, SdpLedgerError> {
        for (declaration_id, current_state) in self.declarations.iter() {
            let Some(params) = service_params.get(&current_state.service_type) else {
                return Err(SdpLedgerError::ServiceParamsNotFound(
                    current_state.service_type,
                ));
            };

            let transient_state =
                TransientDeclarationState::try_from_state(block_number, current_state, params)?;
            let finalized_state = FinalizedDeclarationState::from(&transient_state);

            let Some(forming_session) = self.forming_sessions.get_mut(&current_state.service_type)
            else {
                return Err(SdpLedgerError::FormingSessionNotFound(
                    current_state.service_type,
                ));
            };
            forming_session.update(*declaration_id, &finalized_state);
        }

        self.try_promote(block_number, service_params)
    }

    /// Updates selected service active session with a provider which is set to
    /// active. Should only be used when updating membership during genesis
    /// block.
    pub fn try_update_session_from_genesis(
        mut self,
        block_number: BlockNumber,
        service_type: &ServiceType,
        declaration_id: DeclarationId,
    ) -> Result<Self, SdpLedgerError> {
        if block_number != 0 {
            return Err(SdpLedgerError::NotGenesisBlock);
        }
        let Some(active_session) = self.active_sessions.get_mut(service_type) else {
            return Err(SdpLedgerError::ActiveSessionNotFound(*service_type));
        };
        active_session.update(declaration_id, &FinalizedDeclarationState::Active);

        Ok(self)
    }

    fn try_promote(
        mut self,
        block_number: BlockNumber,
        service_params: &HashMap<ServiceType, ServiceParameters>,
    ) -> Result<Self, SdpLedgerError> {
        let mut new_active_sessions = self.active_sessions;
        let mut new_forming_sessions = self.forming_sessions;

        for service_type in ServiceType::iter() {
            let Some(params) = service_params.get(&service_type) else {
                return Err(SdpLedgerError::SessionParamsNotFound(service_type));
            };

            let Some(forming_session) = new_forming_sessions.get(&service_type) else {
                return Err(SdpLedgerError::FormingSessionNotFound(service_type));
            };

            let expected_active_session_num = block_number / params.session_duration;

            if forming_session.session_number <= expected_active_session_num {
                let new_active_session = forming_session.clone();

                let mut next_forming_session = new_active_session.clone();
                next_forming_session.session_number += 1;
                new_active_sessions = new_active_sessions.insert(service_type, new_active_session);
                new_forming_sessions =
                    new_forming_sessions.insert(service_type, next_forming_session);
            }
        }

        self.active_sessions = new_active_sessions;
        self.forming_sessions = new_forming_sessions;

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::sdp::{ProviderId, ZkPublicKey};
    use num_bigint::BigUint;

    use super::*;
    use crate::cryptarchia::tests::utxo;

    fn setup() -> (SdpLedger, HashMap<ServiceType, ServiceParameters>) {
        let mut params = HashMap::new();
        params.insert(
            ServiceType::BlendNetwork,
            ServiceParameters {
                inactivity_period: 20,
                lock_period: 10,
                retention_period: 1000,
                timestamp: 0,
                session_duration: 10,
            },
        );
        params.insert(
            ServiceType::DataAvailability,
            ServiceParameters {
                inactivity_period: 10,
                lock_period: 5,
                retention_period: 100,
                timestamp: 0,
                session_duration: 5,
            },
        );

        let sdp_ledger = SdpLedger::new();

        (sdp_ledger, params)
    }

    #[test]
    fn test_update_active_provider() {
        let (sdp_ledger, service_params) = setup();
        let service_a = ServiceType::BlendNetwork;
        let block_number = 5;
        let utxo = utxo();
        let note_id = utxo.id();

        let op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId::try_from([0; 32]).unwrap(),
            locators: Vec::new(),
        };
        let declaration_id = op.declaration_id();

        let sdp_ledger = sdp_ledger.apply_declare_msg(5, op).unwrap();

        let updated_membership = sdp_ledger
            .try_update_session(block_number, &service_params)
            .unwrap();

        let forming_session = updated_membership.forming_sessions.get(&service_a).unwrap();
        assert!(forming_session.declarations.contains(&declaration_id));
        assert_eq!(forming_session.declarations.len(), 1);
    }

    #[test]
    fn test_update_inactive_provider() {
        let (sdp_ledger, service_params) = setup();
        let service_a = ServiceType::BlendNetwork;
        let utxo = utxo();
        let note_id = utxo.id();

        let op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId::try_from([0; 32]).unwrap(),
            locators: Vec::new(),
        };
        let declaration_id = op.declaration_id();

        let sdp_ledger = sdp_ledger.apply_declare_msg(5, op).unwrap();

        let with_provider = sdp_ledger.try_update_session(5, &service_params).unwrap();
        assert!(
            with_provider
                .forming_sessions
                .get(&service_a)
                .unwrap()
                .declarations
                .contains(&declaration_id)
        );

        let without_provider = with_provider
            .try_update_session(30, &service_params)
            .unwrap();

        let forming_session = without_provider.forming_sessions.get(&service_a).unwrap();
        assert!(!forming_session.declarations.contains(&declaration_id));
        assert!(forming_session.declarations.is_empty());
    }

    #[test]
    fn test_promote_session_with_updated_provider() {
        let (sdp_ledger, service_params) = setup();
        let service_a = ServiceType::BlendNetwork;
        let utxo = utxo();
        let note_id = utxo.id();

        let op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId::try_from([0; 32]).unwrap(),
            locators: Vec::new(),
        };
        let declaration_id = op.declaration_id();

        let sdp_ledger = sdp_ledger.apply_declare_msg(1, op).unwrap();

        let updated_membership = sdp_ledger.try_update_session(1, &service_params).unwrap();

        let promoted_membership = updated_membership.try_promote(10, &service_params).unwrap();

        let active_session = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session.session_number, 1);
        assert!(active_session.declarations.contains(&declaration_id));

        let forming_session = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session.session_number, 2);
        assert!(forming_session.declarations.contains(&declaration_id));
    }

    #[test]
    fn test_no_promotion() {
        let (sdp_ledger, service_params) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let promoted_membership = sdp_ledger.try_promote(9, &service_params).unwrap();
        let active_session = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session.session_number, 0);
        assert!(active_session.declarations.is_empty());
        let forming_session = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session.session_number, 1);
    }

    #[test]
    fn test_promote_one_service() {
        let (sdp_ledger, service_params) = setup();
        let service_a: ServiceType = ServiceType::BlendNetwork;
        let service_b: ServiceType = ServiceType::DataAvailability;
        let promoted_membership = sdp_ledger.try_promote(6, &service_params).unwrap();
        assert_eq!(
            promoted_membership
                .active_sessions
                .get(&service_b)
                .unwrap()
                .session_number,
            1
        );
        assert_eq!(
            promoted_membership
                .forming_sessions
                .get(&service_b)
                .unwrap()
                .session_number,
            2
        );
        let active_session_a = promoted_membership.active_sessions.get(&service_a).unwrap();
        assert_eq!(active_session_a.session_number, 0);
        let forming_session_a = promoted_membership
            .forming_sessions
            .get(&service_a)
            .unwrap();
        assert_eq!(forming_session_a.session_number, 1);
    }
}
