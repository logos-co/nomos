// Temporary placeholder state for chain-leader
// This will be replaced with proper leader-specific state

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, marker::PhantomData};
use overwatch::services::state::ServiceState;
use nomos_core::header::HeaderId;
use nomos_ledger::LedgerState;
use crate::{CryptarchiaSettings, Error, bootstrap::state::LastEngineState};

#[derive(Clone, Serialize, Deserialize)]
pub struct CryptarchiaConsensusState<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings> {
    pub tip: HeaderId,
    pub lib: HeaderId,
    pub lib_ledger_state: LedgerState,
    pub storage_blocks_to_remove: HashSet<HeaderId>,
    pub last_engine_state: Option<LastEngineState>,
    _markers: PhantomData<(TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings)>,
}

impl<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings>
    CryptarchiaConsensusState<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings>
{
    // Temporary placeholder - will be removed when we refactor leader
    pub fn from_cryptarchia_and_unpruned_blocks(
        _cryptarchia: &crate::Cryptarchia,
        storage_blocks_to_remove: HashSet<HeaderId>,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            tip: [0; 32].into(),
            lib: [0; 32].into(),
            lib_ledger_state: LedgerState::from_utxos([]),
            storage_blocks_to_remove,
            last_engine_state: None,
            _markers: PhantomData,
        })
    }
}

impl<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings> ServiceState
    for CryptarchiaConsensusState<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings>
where
    NodeId: Clone + Eq + std::hash::Hash,
{
    type Settings = CryptarchiaSettings<TxS, NodeId, NetworkAdapterSettings, BlendAdapterSettings>;
    type Error = Error;

    fn from_settings(
        settings: &<Self as ServiceState>::Settings,
    ) -> Result<Self, <Self as ServiceState>::Error> {
        Ok(Self {
            tip: settings.genesis_id,
            lib: settings.genesis_id,
            lib_ledger_state: settings.genesis_state.clone(),
            storage_blocks_to_remove: HashSet::new(),
            last_engine_state: None,
            _markers: PhantomData,
        })
    }
}