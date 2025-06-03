use std::marker::PhantomData;

use cl::NoteWitness;
use cryptarchia_engine::CryptarchiaState;
use nomos_core::header::HeaderId;
use nomos_ledger::LedgerState;
use overwatch::services::state::ServiceState;
use serde::{Deserialize, Serialize};

use crate::{leadership::Leader, Cryptarchia, CryptarchiaSettings, Error};

/// Indicates that there's stored data so [`Cryptarchia`] should be recovered.
/// However, the number of stored epochs is fewer than
/// [`Config::security_param`](cryptarchia_engine::config::Config).
///
/// As a result, a [`Cryptarchia`](cryptarchia_engine::Cryptarchia) instance
/// must first be built from genesis and then recovered up to the `tip` epoch.
pub struct GenesisRecoveryStrategy {
    pub tip: HeaderId,
}

/// Indicates that there's stored data so [`Cryptarchia`] should be recovered,
/// and the number of stored epochs is larger than
/// [`Config::security_param`](cryptarchia_engine::config::Config).
///
/// As a result, a [`Cryptarchia`](cryptarchia_engine::Cryptarchia) instance
/// must first be built from the security state and then recovered up to the
/// `tip` epoch.
pub struct SecurityRecoveryStrategy {
    pub tip: HeaderId,
    pub security_block_id: HeaderId,
    pub security_ledger_state: LedgerState,
    pub security_leader_notes: Vec<NoteWitness>,
    pub security_block_chain_length: u64,
}

pub enum CryptarchiaInitialisationStrategy {
    /// Indicates that there's no stored data so [`Cryptarchia`] should be built
    /// from genesis.
    Genesis,
    RecoveryFromGenesis(GenesisRecoveryStrategy),
    RecoveryFromSecurity(Box<SecurityRecoveryStrategy>),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CryptarchiaConsensusState<
    TxS,
    BxS,
    NetworkAdapterSettings,
    BlendAdapterSettings,
    TimeBackendSettings,
> {
    tip: Option<HeaderId>,
    security_block: Option<HeaderId>,
    security_ledger_state: Option<LedgerState>,
    security_leader_notes: Option<Vec<NoteWitness>>,
    security_block_length: Option<u64>,
    _txs: PhantomData<TxS>,
    _bxs: PhantomData<BxS>,
    _network_adapter_settings: PhantomData<NetworkAdapterSettings>,
    _blend_adapter_settings: PhantomData<BlendAdapterSettings>,
    _time_backend_settings: PhantomData<TimeBackendSettings>,
}

impl<TxS, BxS, NetworkAdapterSettings, BlendAdapterSettings, TimeBackendSettings>
    CryptarchiaConsensusState<
        TxS,
        BxS,
        NetworkAdapterSettings,
        BlendAdapterSettings,
        TimeBackendSettings,
    >
{
    pub const fn new(
        tip: Option<HeaderId>,
        security_block: Option<HeaderId>,
        security_ledger_state: Option<LedgerState>,
        security_leader_notes: Option<Vec<NoteWitness>>,
        security_block_length: Option<u64>,
    ) -> Self {
        Self {
            tip,
            security_block,
            security_ledger_state,
            security_leader_notes,
            security_block_length,
            _txs: PhantomData,
            _bxs: PhantomData,
            _network_adapter_settings: PhantomData,
            _blend_adapter_settings: PhantomData,
            _time_backend_settings: PhantomData,
        }
    }

    pub(crate) fn from_cryptarchia<State: CryptarchiaState>(
        cryptarchia: &Cryptarchia<State>,
        leader: &Leader,
    ) -> Self {
        let security_block_header = cryptarchia.consensus.get_security_block_header_id();
        let security_ledger_state = security_block_header
            .and_then(|header| cryptarchia.ledger.state(&header))
            .cloned();
        let security_block_length = security_block_header.and_then(|header| {
            cryptarchia
                .consensus
                .branches()
                .get_length_for_header(&header)
        });
        let security_leader_notes = security_block_header
            .and_then(|header_id| leader.notes(&header_id))
            .map(Vec::from);

        Self::new(
            Some(cryptarchia.tip()),
            security_block_header,
            security_ledger_state,
            security_leader_notes,
            security_block_length,
        )
    }

    const fn can_recover(&self) -> bool {
        // This only checks whether tip is defined, as that's a state variable that
        // should always exist for recovery. Other attributes might not be present.
        self.tip.is_some()
    }

    const fn can_recover_from_security(&self) -> bool {
        self.can_recover()
            && self.security_block.is_some()
            && self.security_ledger_state.is_some()
            && self.security_leader_notes.is_some()
            && self.security_block_length.is_some()
    }

    pub fn recovery_strategy(&mut self) -> CryptarchiaInitialisationStrategy {
        if self.can_recover_from_security() {
            let strategy = SecurityRecoveryStrategy {
                tip: self.tip.take().expect("tip not available"),
                security_block_id: self
                    .security_block
                    .take()
                    .expect("security block not available"),
                security_ledger_state: self
                    .security_ledger_state
                    .take()
                    .expect("security ledger state not available"),
                security_leader_notes: self
                    .security_leader_notes
                    .take()
                    .expect("security leader notes not available"),
                security_block_chain_length: self
                    .security_block_length
                    .take()
                    .expect("security block length not available"),
            };
            CryptarchiaInitialisationStrategy::RecoveryFromSecurity(Box::new(strategy))
        } else if self.can_recover() {
            let strategy = GenesisRecoveryStrategy {
                tip: self.tip.expect("tip not available"),
            };
            CryptarchiaInitialisationStrategy::RecoveryFromGenesis(strategy)
        } else {
            CryptarchiaInitialisationStrategy::Genesis
        }
    }
}

impl<TxS, BxS, NetworkAdapterSettings, BlendAdapterSettings, TimeBackendSettings> ServiceState
    for CryptarchiaConsensusState<
        TxS,
        BxS,
        NetworkAdapterSettings,
        BlendAdapterSettings,
        TimeBackendSettings,
    >
{
    type Settings = CryptarchiaSettings<TxS, BxS, NetworkAdapterSettings, BlendAdapterSettings>;
    type Error = Error;

    fn from_settings(_settings: &Self::Settings) -> Result<Self, Self::Error> {
        Ok(Self::new(None, None, None, None, None))
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::{Debug, Formatter};

    use super::*;

    impl PartialEq for CryptarchiaInitialisationStrategy {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Genesis, Self::Genesis) => true,
                (Self::RecoveryFromGenesis(a), Self::RecoveryFromGenesis(b)) => a == b,
                (Self::RecoveryFromSecurity(a), Self::RecoveryFromSecurity(b)) => a == b,
                _ => false,
            }
        }
    }

    impl Debug for CryptarchiaInitialisationStrategy {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Genesis => f.debug_tuple("Genesis").finish(),
                Self::RecoveryFromGenesis(strategy) => f
                    .debug_tuple("RecoveryFromGenesis")
                    .field(strategy)
                    .finish(),
                Self::RecoveryFromSecurity(strategy) => f
                    .debug_tuple("RecoveryFromSecurity")
                    .field(strategy)
                    .finish(),
            }
        }
    }

    impl PartialEq for GenesisRecoveryStrategy {
        fn eq(&self, other: &Self) -> bool {
            self.tip == other.tip
        }
    }

    impl Debug for GenesisRecoveryStrategy {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("GenesisRecoveryStrategy")
                .field("tip", &self.tip)
                .finish()
        }
    }

    impl PartialEq for SecurityRecoveryStrategy {
        fn eq(&self, other: &Self) -> bool {
            self.tip == other.tip
                && self.security_block_id == other.security_block_id
                && self.security_ledger_state == other.security_ledger_state
                && self.security_leader_notes == other.security_leader_notes
        }
    }

    impl Debug for SecurityRecoveryStrategy {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SecurityRecoveryStrategy")
                .field("tip", &self.tip)
                .field("security_block_id", &self.security_block_id)
                .field("security_ledger_state", &self.security_ledger_state)
                .field("security_leader_notes", &self.security_leader_notes)
                .field("security_block_length", &self.security_block_chain_length)
                .finish()
        }
    }

    #[test]
    fn test_can_recover() {
        let state =
            CryptarchiaConsensusState::<(), (), (), (), ()>::new(None, None, None, None, None);
        assert!(!state.can_recover());

        let header_id = HeaderId::from([0; 32]);
        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            None,
            None,
            None,
            None,
        );
        assert!(state.can_recover());
    }

    #[test]
    fn test_can_recover_from_security() {
        let header_id = HeaderId::from([0; 32]);
        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            None,
            None,
            None,
            None,
        );
        assert!(!state.can_recover_from_security());

        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            Some(header_id),
            Some(LedgerState::from_commitments(vec![], 0)),
            Some(Vec::new()),
            Some(0),
        );
        assert!(state.can_recover_from_security());
    }

    #[test]
    fn test_recovery_strategy() {
        let mut state =
            CryptarchiaConsensusState::<(), (), (), (), ()>::new(None, None, None, None, None);
        assert_eq!(
            state.recovery_strategy(),
            CryptarchiaInitialisationStrategy::Genesis
        );

        let header_id = HeaderId::from([0; 32]);
        let mut state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            state.recovery_strategy(),
            CryptarchiaInitialisationStrategy::RecoveryFromGenesis(GenesisRecoveryStrategy {
                tip: header_id
            })
        );

        let ledger_state = LedgerState::from_commitments(vec![], 0);
        let mut state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            Some(header_id),
            Some(ledger_state.clone()),
            Some(Vec::new()),
            Some(0),
        );
        assert_eq!(
            state.recovery_strategy(),
            CryptarchiaInitialisationStrategy::RecoveryFromSecurity(Box::new(
                SecurityRecoveryStrategy {
                    tip: header_id,
                    security_block_id: header_id,
                    security_ledger_state: ledger_state,
                    security_leader_notes: Vec::new(),
                    security_block_chain_length: 0,
                }
            ))
        );
    }
}
