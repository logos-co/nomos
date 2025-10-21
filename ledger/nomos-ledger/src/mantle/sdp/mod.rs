pub mod locked_notes;

use std::collections::HashMap;

use ed25519::{Signature as Ed25519Sig, signature::Verifier as _};
use locked_notes::LockedNotes;
use nomos_core::{
    block::BlockNumber,
    mantle::{
        Note, TxHash,
        ops::sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
    },
    proofs::zksig::{ZkSignatureProof, ZkSignaturePublic},
    sdp::{
        Declaration, DeclarationId, MinStake, Nonce, ProviderId, ProviderInfo, ServiceParameters,
        ServiceType, SessionNumber,
    },
};

type Declarations = rpds::HashTrieMapSync<DeclarationId, Declaration>;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub service_params: std::sync::Arc<HashMap<ServiceType, ServiceParameters>>,
    pub min_stake: MinStake,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    // #[error("Invalid Sdp state transition: {0:?}")]
    // SdpStateError(#[from] DeclarationStateError),
    #[error("Sdp declaration id not found: {0:?}")]
    DeclarationNotFound(DeclarationId),
    #[error("Locked period did not pass yet")]
    WithdrawalWhileLocked,
    #[error("Invalid sdp message nonce: {0:?}")]
    InvalidNonce(Nonce),
    #[error("Service not found: {0:?}")]
    ServiceNotFound(ServiceType),
    #[error("Duplicate sdp declaration id: {0:?}")]
    DuplicateDeclaration(DeclarationId),
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
    #[error("Time travel detected, current: {current:?}, incoming: {incoming:?}")]
    TimeTravel {
        current: BlockNumber,
        incoming: BlockNumber,
    },
    #[error("Something went wrong while locking/unlocking a note: {0:?}")]
    LockingError(#[from] locked_notes::Error),
    #[error("Invalid signature")]
    InvalidSignature,
}

// State at the beginning of this session
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionState {
    declarations: Declarations,
    session_n: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceState {
    // state of declarations at block b
    declarations: Declarations,
    // (current) active session
    // snapshot of `declarations` at the start of block ((b // config.session_duration) - 1) *
    // config.session_duration
    active: SessionState,
    // he past session, used for rewards
    // snapshot of `declarations` at the start of block ((b // config.session_duration) - 2) *
    // config.session_duration
    past_session: SessionState,
    // new forming session, overlaps with `declarations` until the next session boundary
    // snapshot of `declarations` at the start of block (b // config.session_duration) *
    // config.session_duration
    forming: SessionState,
}

impl SessionState {
    fn update(
        &self,
        service_state: &ServiceState,
        block_number: u64,
        config: &ServiceParameters,
    ) -> Self {
        if self.session_n.saturating_sub(1) * config.session_duration > block_number {
            return Self {
                session_n: self.session_n,
                declarations: service_state.declarations.clone(),
            };
        }
        self.clone()
    }
}

impl ServiceState {
    fn try_apply_header(mut self, block_number: u64, config: &ServiceParameters) -> Self {
        let current_session = block_number / config.session_duration;
        // shift all session!
        if current_session == self.active.session_n + 1 {
            // TODO: distribute rewards
            self.past_session = self.active.clone();
            self.active = self.forming.clone();
            self.forming = SessionState {
                declarations: self.declarations.clone(),
                session_n: self.forming.session_n + 1,
            };
        } else {
            self.forming = self.forming.update(&self, block_number, config);
        }
        self
    }

    fn declare(&mut self, id: DeclarationId, declaration: Declaration) -> Result<(), Error> {
        if self.declarations.contains_key(&id) {
            return Err(Error::DuplicateDeclaration(id));
        }
        self.declarations = self.declarations.insert(id, declaration);
        Ok(())
    }

    fn active(
        &mut self,
        active: &SDPActiveOp,
        block_number: BlockNumber,
        locked_notes: &LockedNotes,
        sig: &impl ZkSignatureProof,
        tx_hash: TxHash,
    ) -> Result<(), Error> {
        let Some(declaration) = self.declarations.get_mut(&active.declaration_id) else {
            return Err(Error::DeclarationNotFound(active.declaration_id));
        };

        if active.nonce <= declaration.nonce {
            return Err(Error::InvalidNonce(active.nonce));
        }
        declaration.active = block_number;
        let note = locked_notes
            .get(&declaration.locked_note_id)
            .ok_or(Error::LockingError(locked_notes::Error::NoteNotLocked(
                declaration.locked_note_id,
            )))?;

        if !sig.verify(&ZkSignaturePublic {
            pks: vec![note.pk.into(), declaration.zk_id.0],
            msg_hash: tx_hash.0,
        }) {
            return Err(Error::InvalidSignature);
        }

        // TODO: check service specific logic

        Ok(())
    }

    fn withdraw(
        &mut self,
        withdraw: &SDPWithdrawOp,
        block_number: BlockNumber,
        locked_notes: &mut LockedNotes,
        sig: &impl ZkSignatureProof,
        tx_hash: TxHash,
        config: &ServiceParameters,
    ) -> Result<(), Error> {
        let Some(declaration) = self.declarations.get(&withdraw.declaration_id) else {
            return Err(Error::DeclarationNotFound(withdraw.declaration_id));
        };
        if withdraw.nonce <= declaration.nonce {
            return Err(Error::InvalidNonce(withdraw.nonce));
        }

        if declaration.created + config.lock_period >= block_number {
            return Err(Error::WithdrawalWhileLocked);
        }

        let note = locked_notes.unlock(declaration.service_type, &declaration.locked_note_id)?;

        if !sig.verify(&ZkSignaturePublic {
            pks: vec![note.pk.into(), declaration.zk_id.0],
            msg_hash: tx_hash.0,
        }) {
            return Err(Error::InvalidSignature);
        }
        self.declarations = self.declarations.remove(&withdraw.declaration_id);
        Ok(())
    }

    fn contains(&self, declaration_id: &DeclarationId) -> bool {
        self.declarations.contains_key(declaration_id)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SdpLedger {
    services: rpds::HashTrieMapSync<ServiceType, ServiceState>,
    locked_notes: LockedNotes,
    block_number: u64,
}

impl SdpLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: rpds::HashTrieMapSync::new_sync(),
            locked_notes: LockedNotes::new(),
            block_number: 0,
        }
    }

    // TODO: genesis init
    #[cfg(test)]
    #[must_use]
    fn with_service(mut self, service_type: ServiceType) -> Self {
        let service_state = ServiceState {
            declarations: rpds::HashTrieMapSync::new_sync(),
            active: SessionState {
                declarations: rpds::HashTrieMapSync::new_sync(),
                session_n: 0,
            },
            past_session: SessionState {
                declarations: rpds::HashTrieMapSync::new_sync(),
                session_n: 0,
            },
            forming: SessionState {
                declarations: rpds::HashTrieMapSync::new_sync(),
                session_n: 1,
            },
        };
        self.services = self.services.insert(service_type, service_state);
        self
    }

    pub fn try_apply_header(&self, config: &Config) -> Result<Self, Error> {
        let block_number = self.block_number + 1; // overflow?
        Ok(Self {
            block_number,
            services: self
                .services
                .iter()
                .map(|(service, service_state)| {
                    let config = config
                        .service_params
                        .get(service)
                        .ok_or(Error::SessionParamsNotFound(*service))?;
                    Ok::<_, Error>((
                        *service,
                        service_state.clone().try_apply_header(block_number, config),
                    ))
                })
                .collect::<Result<_, _>>()?,
            locked_notes: self.locked_notes.clone(),
        })
    }

    pub fn apply_declare_msg(
        mut self,
        op: &SDPDeclareOp,
        note: Note,
        zk_sig: &impl ZkSignatureProof,
        ed25519_sig: &Ed25519Sig,
        tx_hash: TxHash,
        config: &Config,
    ) -> Result<Self, Error> {
        if !zk_sig.verify(&ZkSignaturePublic {
            pks: vec![note.pk.into(), op.zk_id.0],
            msg_hash: tx_hash.0,
        }) {
            return Err(Error::InvalidSignature);
        }
        op.provider_id
            .0
            .verify(tx_hash.as_signing_bytes().as_ref(), ed25519_sig)
            .map_err(|_| Error::InvalidSignature)?;

        let declaration_id = op.id();
        let declaration = Declaration::new(self.block_number, op);
        if let Some(service_state) = self.services.get_mut(&op.service_type) {
            service_state.declare(declaration_id, declaration)?;
            self.locked_notes = self.locked_notes.lock(
                &config.min_stake,
                op.service_type,
                note,
                &op.locked_note_id,
            )?;
        } else {
            return Err(Error::ServiceNotFound(op.service_type));
        }

        Ok(self)
    }

    pub fn apply_active_msg(
        mut self,
        op: &SDPActiveOp,
        zksig: &impl ZkSignatureProof,
        tx_hash: TxHash,
        config: &Config,
    ) -> Result<Self, Error> {
        let (service, _) = self.get_service(&op.declaration_id, config)?;
        self.services.get_mut(&service).unwrap().active(
            op,
            self.block_number,
            &self.locked_notes,
            zksig,
            tx_hash,
        )?;

        Ok(self)
    }

    pub fn apply_withdrawn_msg(
        mut self,
        op: &SDPWithdrawOp,
        zksig: &impl ZkSignatureProof,
        tx_hash: TxHash,
        config: &Config,
    ) -> Result<Self, Error> {
        let (service, config) = self.get_service(&op.declaration_id, config)?;
        self.services.get_mut(&service).unwrap().withdraw(
            op,
            self.block_number,
            &mut self.locked_notes,
            zksig,
            tx_hash,
            config,
        )?;

        Ok(self)
    }

    #[must_use]
    pub const fn locked_notes(&self) -> &LockedNotes {
        &self.locked_notes
    }

    #[must_use]
    pub fn active_session_providers(
        &self,
        service_type: ServiceType,
        config: &Config,
    ) -> Option<HashMap<ProviderId, ProviderInfo>> {
        let service_state = self.services.get(&service_type)?;
        let service_params = config.service_params.get(&service_type)?;

        let providers = service_state
            .active
            .declarations
            .iter()
            .filter_map(|(_, declaration)| {
                // Check if provider is still active
                // A provider is active if: declaration.active + inactivity_period >=
                // current_block_number
                #[expect(clippy::if_then_some_else_none, reason = "suggestion is ugly")]
                if declaration.active + service_params.inactivity_period >= self.block_number {
                    Some((
                        declaration.provider_id,
                        ProviderInfo {
                            locators: declaration.locators.clone(),
                            zk_id: declaration.zk_id,
                        },
                    ))
                } else {
                    None
                }
            })
            .collect();

        Some(providers)
    }

    #[must_use]
    pub fn active_sessions(&self) -> HashMap<ServiceType, SessionNumber> {
        self.services
            .iter()
            .map(|(service_type, service_state)| (*service_type, service_state.active.session_n))
            .collect()
    }

    fn get_service<'a>(
        &self,
        declaration_id: &DeclarationId,
        config: &'a Config,
    ) -> Result<(ServiceType, &'a ServiceParameters), Error> {
        let service = self
            .services
            .iter()
            .find(|(_, state)| state.contains(declaration_id))
            .map(|(service, _)| *service)
            .ok_or(Error::DeclarationNotFound(*declaration_id))?;

        let params = config
            .service_params
            .get(&service)
            .ok_or(Error::ServiceParamsNotFound(service))?;
        Ok((service, params))
    }

    #[cfg(test)]
    fn get_forming_session(&self, service_type: ServiceType) -> Option<&SessionState> {
        self.services.get(&service_type).map(|s| &s.forming)
    }

    #[cfg(test)]
    fn get_active_session(&self, service_type: ServiceType) -> Option<&SessionState> {
        self.services.get(&service_type).map(|s| &s.active)
    }

    #[cfg(test)]
    fn get_declarations(
        &self,
        service_type: ServiceType,
    ) -> Option<&rpds::HashTrieMapSync<DeclarationId, Declaration>> {
        self.services.get(&service_type).map(|s| &s.declarations)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::{Signer as _, SigningKey};
    use groth16::Fr;
    use nomos_core::{proofs::zksig::DummyZkSignature, sdp::ZkPublicKey};
    use num_bigint::BigUint;

    use super::*;
    use crate::cryptarchia::tests::utxo;

    fn setup() -> Config {
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

        Config {
            service_params: Arc::new(params),
            min_stake: MinStake {
                threshold: 1,
                timestamp: 0,
            },
        }
    }

    fn create_dummy_zk_sig(note_pk: Fr, zk_id: Fr, tx_hash: Fr) -> DummyZkSignature {
        DummyZkSignature::prove(&ZkSignaturePublic {
            pks: vec![note_pk, zk_id],
            msg_hash: tx_hash,
        })
    }

    fn create_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0; 32])
    }

    fn apply_declare_with_dummies(
        sdp_ledger: SdpLedger,
        op: &SDPDeclareOp,
        config: &Config,
    ) -> Result<SdpLedger, Error> {
        let utxo = utxo();
        let note = utxo.note;
        let tx_hash = TxHash(Fr::from(0u8));
        let zk_sig = create_dummy_zk_sig(note.pk.into(), op.zk_id.0, tx_hash.0);
        let signing_key = create_signing_key();
        let ed25519_sig = signing_key.sign(tx_hash.as_signing_bytes().as_ref());

        sdp_ledger.apply_declare_msg(op, note, &zk_sig, &ed25519_sig, tx_hash, config)
    }

    fn apply_withdraw_with_dummies(
        sdp_ledger: SdpLedger,
        op: &SDPWithdrawOp,
        note_pk: Fr,
        zk_id: Fr,
        config: &Config,
    ) -> Result<SdpLedger, Error> {
        let tx_hash = TxHash(Fr::from(1u8));
        let zk_sig = create_dummy_zk_sig(note_pk, zk_id, tx_hash.0);

        sdp_ledger.apply_withdrawn_msg(op, &zk_sig, tx_hash, config)
    }

    #[test]
    fn test_update_active_provider() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let utxo = utxo();
        let note_id = utxo.id();
        let signing_key = create_signing_key();

        let op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id = op.id();

        // Initialize ledger with service config
        let sdp_ledger = SdpLedger::new().with_service(service_a);

        // Apply declare at block 0
        let sdp_ledger = apply_declare_with_dummies(sdp_ledger, op, &config).unwrap();

        // Declaration is in service_state.declarations but not in sessions yet
        let declarations = sdp_ledger.get_declarations(service_a).unwrap();
        assert!(declarations.contains_key(&declaration_id));

        // Apply headers to reach block 10 (session boundary)
        let mut sdp_ledger = sdp_ledger;
        for _ in 0..10 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // At block 10, declaration enters forming session 2
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 2);
        assert!(forming_session.declarations.contains_key(&declaration_id));
        assert_eq!(forming_session.declarations.size(), 1);
    }

    #[test]
    fn test_withdraw_provider() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let utxo = utxo();
        let note_id = utxo.id();
        let signing_key = create_signing_key();

        let declare_op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id = declare_op.id();

        // Initialize ledger with service config and declare
        let sdp_ledger = SdpLedger::new().with_service(service_a);

        let sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op, &config).unwrap();

        // Verify declaration is present
        let declarations = sdp_ledger.get_declarations(service_a).unwrap();
        assert!(declarations.contains_key(&declaration_id));

        // Move forward enough blocks to satisfy lock_period
        let mut sdp_ledger = sdp_ledger;
        for _ in 0..11 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // Withdraw the declaration
        let withdraw_op = &SDPWithdrawOp {
            declaration_id,
            nonce: 1,
        };
        let sdp_ledger = apply_withdraw_with_dummies(
            sdp_ledger,
            withdraw_op,
            utxo.note.pk.into(),
            declare_op.zk_id.0,
            &config,
        )
        .unwrap();

        // Verify declaration is removed
        let declarations = sdp_ledger.get_declarations(service_a).unwrap();
        assert!(!declarations.contains_key(&declaration_id));
        assert!(declarations.is_empty());
    }

    #[test]
    fn test_promote_session_with_updated_provider() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let utxo = utxo();
        let note_id = utxo.id();
        let signing_key = create_signing_key();

        let op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: note_id,
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id = op.id();

        // Initialize ledger with service config
        let sdp_ledger = SdpLedger::new().with_service(service_a);

        // Declare at block 0
        let sdp_ledger = apply_declare_with_dummies(sdp_ledger, op, &config).unwrap();

        // Apply headers to reach block 10 (session boundary for session_duration=10)
        let mut sdp_ledger = sdp_ledger;
        for _ in 0..10 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // At block 10: active becomes session 1 (was empty forming), forming becomes
        // session 2 (snapshot at block 10)
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 1);
        assert!(active_session.declarations.is_empty()); // Active session 1 is empty

        // Check forming session is now session 2 and contains declaration
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 2);
        assert!(forming_session.declarations.contains_key(&declaration_id));

        // Continue to block 20 to see declaration become active
        for _ in 0..10 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // At block 20: active becomes session 2 (with declaration)
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 2);
        assert!(active_session.declarations.contains_key(&declaration_id));
    }

    #[test]
    fn test_no_promotion() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;

        // Initialize ledger with service config
        let mut sdp_ledger = SdpLedger::new().with_service(service_a);

        // Apply headers to reach block 9 (still in session 0, promotion happens at
        // block 10)
        for _ in 0..9 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // Check active session is still session 0 with no declarations
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 0);
        assert!(active_session.declarations.is_empty());

        // Check forming session is still session 1
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 1);
    }

    #[test]
    fn test_promote_one_service() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork; // session_duration = 10
        let service_b = ServiceType::DataAvailability; // session_duration = 5

        // Initialize ledger with both services
        let mut sdp_ledger = SdpLedger::new()
            .with_service(service_a)
            .with_service(service_b);

        // Apply headers to reach block 6
        // At block 5, DataAvailability promotes (5/5=1, active.session_n=0, so 1==0+1)
        // At block 6, BlendNetwork hasn't promoted yet (6/10=0, active.session_n=0, so
        // 0!=0+1)
        for _ in 0..6 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // Check DataAvailability is promoted to session 1
        let active_session_b = sdp_ledger.get_active_session(service_b).unwrap();
        assert_eq!(active_session_b.session_n, 1);
        let forming_session_b = sdp_ledger.get_forming_session(service_b).unwrap();
        assert_eq!(forming_session_b.session_n, 2);

        // Check BlendNetwork is still in session 0
        let active_session_a = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session_a.session_n, 0);
        let forming_session_a = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session_a.session_n, 1);
    }

    #[test]
    fn test_new_declarations_becoming_active_after_session_boundary() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let signing_key = create_signing_key();

        // Initialize ledger
        let mut sdp_ledger = SdpLedger::new().with_service(service_a);

        // SESSION 0: Add a declaration at block 5
        for _ in 0..5 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        let declare_op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id = declare_op.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op, &config).unwrap();

        // Move to block 9 (last block of session 0)
        for _ in 6..10 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        assert_eq!(sdp_ledger.block_number, 9);

        // Declaration is not in active or forming sessions yet
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 0);
        assert!(!active_session.declarations.contains_key(&declaration_id));

        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 1);
        assert!(forming_session.declarations.is_empty());

        // SESSION 1: Cross session boundary to block 10
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 10);

        // Active session 1 is empty (was the empty forming session 1)
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 1);
        assert!(active_session.declarations.is_empty());

        // Forming session 2 now has the declaration (snapshot from block 10)
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 2);
        assert!(forming_session.declarations.contains_key(&declaration_id));

        // SESSION 2: Cross to block 20
        for _ in 11..20 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 20);

        // Now the declaration is active in session 2
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 2);
        assert!(active_session.declarations.contains_key(&declaration_id));
    }

    #[test]
    fn test_declaration_snapshot_timing() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let signing_key = create_signing_key();

        let mut sdp_ledger = SdpLedger::new().with_service(service_a);

        // Add declaration at block 0
        let declare_op_1 = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(1u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id_1 = declare_op_1.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op_1, &config).unwrap();

        // Move to block 9 (last block before session boundary)
        for _ in 1..10 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        // Save state at block 9
        let sdp_ledger_block_9 = sdp_ledger.clone();

        // Add another declaration at block 10 (after session boundary)
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 10);

        let declare_op_2 = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(2u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id_2 = declare_op_2.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op_2, &config).unwrap();

        // Jump to session 2 (block 20)
        for _ in 11..20 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();

        // Active session (session 2) should contain both declarations
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert!(active_session.declarations.contains_key(&declaration_id_1));
        assert!(!active_session.declarations.contains_key(&declaration_id_2));

        // Now test from the block 9 state - jumping directly to block 20
        let mut sdp_ledger_from_9 = sdp_ledger_block_9;
        for _ in 10..20 {
            sdp_ledger_from_9 = sdp_ledger_from_9.try_apply_header(&config).unwrap();
        }
        sdp_ledger_from_9 = sdp_ledger_from_9.try_apply_header(&config).unwrap();

        // Active session should only contain declaration_id_1
        // because declaration_id_2 was never added in this timeline
        let active_session_from_9 = sdp_ledger_from_9.get_active_session(service_a).unwrap();
        assert!(
            active_session_from_9
                .declarations
                .contains_key(&declaration_id_1)
        );
        assert!(
            !active_session_from_9
                .declarations
                .contains_key(&declaration_id_2)
        );
    }

    #[test]
    fn test_session_jump() {
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let signing_key = create_signing_key();

        let mut sdp_ledger = SdpLedger::new().with_service(service_a);

        // Add declaration at block 3
        for _ in 0..3 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }

        let declare_op = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(0u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id = declare_op.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op, &config).unwrap();

        // Jump directly from block 3 to block 25 (skipping session 1 entirely)
        for _ in 4..25 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 25);

        // Declaration snapshots should be taken from the last known state
        // Active session (session 2, which started at block 20) should contain the
        // declaration
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 2);
        assert!(active_session.declarations.contains_key(&declaration_id));

        // Forming session (session 3) should also contain the declaration
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 3);
        assert!(forming_session.declarations.contains_key(&declaration_id));
    }

    #[test]
    #[expect(clippy::cognitive_complexity, reason = "sessions are complex :)")]
    fn test_session_boundary() {
        // Test a declaration at block 9 is available in session 2 but a declaration in
        // block 10 is not
        let config = setup();
        let service_a = ServiceType::BlendNetwork;
        let signing_key = create_signing_key();

        let mut sdp_ledger = SdpLedger::new().with_service(service_a);

        // Move to block 9 (last block of session 0)
        for _ in 0..9 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        assert_eq!(sdp_ledger.block_number, 9);

        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 0);
        assert!(active_session.declarations.is_empty());

        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 1);
        assert!(forming_session.declarations.is_empty());

        // Create first declaration at block 9
        let declare_op_1 = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(1u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id_1 = declare_op_1.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op_1, &config).unwrap();

        // Cross to block 10 (session boundary - start of session 1)
        // At this point, the snapshot for forming session 2 is taken
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 10);

        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 1);
        assert!(active_session.declarations.is_empty());

        // Forming session 2 should contain declaration_1 (made at block 9)
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 2);
        assert!(forming_session.declarations.contains_key(&declaration_id_1));

        // Create second declaration at block 10 (first block of session 1)
        let declare_op_2 = &SDPDeclareOp {
            service_type: service_a,
            locked_note_id: utxo().id(),
            zk_id: ZkPublicKey(BigUint::from(2u8).into()),
            provider_id: ProviderId(signing_key.verifying_key()),
            locators: Vec::new(),
        };
        let declaration_id_2 = declare_op_2.id();

        sdp_ledger = apply_declare_with_dummies(sdp_ledger, declare_op_2, &config).unwrap();

        // Forming session 2 still only has declaration_1 (snapshot was already taken at
        // block 10)
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 2);
        assert!(forming_session.declarations.contains_key(&declaration_id_1));
        assert!(!forming_session.declarations.contains_key(&declaration_id_2));

        // Jump to block 20 (start of session 2)
        for _ in 11..20 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 20);

        // Active session 2 has declaration_1 (from block 9)
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 2);
        assert!(active_session.declarations.contains_key(&declaration_id_1));
        assert!(!active_session.declarations.contains_key(&declaration_id_2));

        // Forming session 3 has both declarations (snapshot from block 20)
        let forming_session = sdp_ledger.get_forming_session(service_a).unwrap();
        assert_eq!(forming_session.session_n, 3);
        assert!(forming_session.declarations.contains_key(&declaration_id_1));
        assert!(forming_session.declarations.contains_key(&declaration_id_2));

        // Jump to block 30 (start of session 3)
        for _ in 21..30 {
            sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        }
        sdp_ledger = sdp_ledger.try_apply_header(&config).unwrap();
        assert_eq!(sdp_ledger.block_number, 30);

        // Active session 3 now has both declarations
        let active_session = sdp_ledger.get_active_session(service_a).unwrap();
        assert_eq!(active_session.session_n, 3);
        assert!(active_session.declarations.contains_key(&declaration_id_1));
        assert!(active_session.declarations.contains_key(&declaration_id_2));
    }
}
