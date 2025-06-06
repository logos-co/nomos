use std::{collections::HashSet, marker::PhantomData};

use cl::NoteWitness;
use cryptarchia_engine::{CryptarchiaState, ForkDivergenceInfo};
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
    /// Set of blocks that have been pruned from the engine but have not yet
    /// been deleted from the persistence layer because of some unexpected
    /// error.
    pub(crate) prunable_blocks: HashSet<HeaderId>,
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
        prunable_blocks: HashSet<HeaderId>,
    ) -> Self {
        Self {
            tip,
            security_block,
            security_ledger_state,
            security_leader_notes,
            security_block_length,
            prunable_blocks,
            _txs: PhantomData,
            _bxs: PhantomData,
            _network_adapter_settings: PhantomData,
            _blend_adapter_settings: PhantomData,
            _time_backend_settings: PhantomData,
        }
    }

    /// Re-create the cryptarchia state given the engine instance and the leader
    /// details.
    ///
    /// Furthermore, it allows to specify blocks deleted from the cryptarchia
    /// engine (hence not tracked anymore) but that should be deleted from the
    /// persistence layer, which are added to the prunable blocks belonging to
    /// old enough forks as returned by the cryptarchia engine.
    pub(crate) fn from_cryptarchia_and_unpruned_blocks<State: CryptarchiaState>(
        cryptarchia: &Cryptarchia<State>,
        leader: &Leader,
        mut prunable_blocks: HashSet<HeaderId>,
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

        // Retrieve the prunable forks from the cryptarchia engine.
        let prunable_forks = security_block_length.map_or_else(Vec::new, |security_block_length| {
            let Some(pruning_depth) = cryptarchia
                .consensus
                .tip_branch()
                .length()
                .checked_sub(security_block_length) else {
                    panic!("The provided security block has a length greater than the tip of the canonical chain, which indicates something is corrupted.");
                };
            cryptarchia
                .consensus
                .prunable_forks(pruning_depth)
                .collect()
        });

        // Merge all blocks from each prunable fork's tip up until (but excluding) the
        // fork's LCA with the canonical chain.
        for ForkDivergenceInfo { lca, tip } in prunable_forks {
            let mut cursor = tip;
            while cursor != lca {
                prunable_blocks.insert(cursor.id());
                cursor = cryptarchia
                    .consensus
                    .branches()
                    .get(&cursor.parent())
                    .copied()
                    .expect("Fork block should have a parent.");
            }
        }

        Self::new(
            Some(cryptarchia.tip()),
            security_block_header,
            security_ledger_state,
            security_leader_notes,
            security_block_length,
            prunable_blocks,
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
        Ok(Self::new(None, None, None, None, None, HashSet::new()))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::{Debug, Formatter},
        num::NonZero,
    };

    use cl::NullifierSecret;
    use cryptarchia_engine::Boostrapping;

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
        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            None,
            None,
            None,
            None,
            None,
            HashSet::new(),
        );
        assert!(!state.can_recover());

        let header_id = HeaderId::from([0; 32]);
        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            None,
            None,
            None,
            None,
            HashSet::new(),
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
            HashSet::new(),
        );
        assert!(!state.can_recover_from_security());

        let state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            Some(header_id),
            Some(header_id),
            Some(LedgerState::from_commitments(vec![], 0)),
            Some(Vec::new()),
            Some(0),
            HashSet::new(),
        );
        assert!(state.can_recover_from_security());
    }

    #[test]
    fn test_recovery_strategy() {
        let mut state = CryptarchiaConsensusState::<(), (), (), (), ()>::new(
            None,
            None,
            None,
            None,
            None,
            HashSet::new(),
        );
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
            HashSet::new(),
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
            HashSet::new(),
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

    #[test]
    fn save_prunable_forks() {
        let genesis_header_id: HeaderId = [0; 32].into();
        // We don't prune fork stemming from the block before the current tip.
        let security_param: NonZero<u32> = 2.try_into().unwrap();
        let cryptarchia_engine_config = cryptarchia_engine::Config {
            security_param,
            active_slot_coeff: 0f64,
        };
        let ledger_config = nomos_ledger::Config {
            epoch_config: cryptarchia_engine::EpochConfig {
                epoch_stake_distribution_stabilization: 1.try_into().unwrap(),
                epoch_period_nonce_buffer: 1.try_into().unwrap(),
                epoch_period_nonce_stabilization: 1.try_into().unwrap(),
            },
            consensus_config: cryptarchia_engine_config,
        };

        let cryptarchia_engine = {
            // Boostrapping mode since we are pursposefully adding old forks to test the
            // recovery mechanism.
            let mut cryptarchia = cryptarchia_engine::Cryptarchia::<_, Boostrapping>::from_genesis(
                genesis_header_id,
                cryptarchia_engine_config,
            );

            // Add 3 more blocks to canonical chain. Blocks `0`, `1`, `2` and `3` represent
            // the canonical chain now.
            cryptarchia = cryptarchia
                .receive_block([1; 32].into(), genesis_header_id, 1.into())
                .expect("Block 1 to be added successfully on top of block 0.")
                .receive_block([2; 32].into(), [1; 32].into(), 2.into())
                .expect("Block 2 to be added successfully on top of block 1.")
                .receive_block([3; 32].into(), [2; 32].into(), 3.into())
                .expect("Block 3 to be added successfully on top of block 2.");
            // Add a 2-block fork from genesis
            cryptarchia = cryptarchia
                .receive_block([4; 32].into(), genesis_header_id, 1.into())
                .expect("Block 4 to be added successfully on top of block 0.")
                .receive_block([5; 32].into(), [4; 32].into(), 2.into())
                .expect("Block 5 to be added successfully on top of block 4.");
            // Add a second single-block fork from genesis
            cryptarchia = cryptarchia
                .receive_block([6; 32].into(), genesis_header_id, 1.into())
                .expect("Block 6 to be added successfully on top of block 0.");
            // Add a single-block fork from the block after genesis (block `1`)
            cryptarchia = cryptarchia
                .receive_block([7; 32].into(), [1; 32].into(), 2.into())
                .expect("Block 7 to be added successfully on top of block 1.");
            // Add a single-block fork from the second block after genesis (block `2`)
            cryptarchia = cryptarchia
                .receive_block([8; 32].into(), [2; 32].into(), 3.into())
                .expect("Block 8 to be added successfully on top of block 2.");

            cryptarchia.online()
        };
        // Empty ledger state.
        let ledger_state = nomos_ledger::Ledger::new(
            [0; 32].into(),
            nomos_ledger::LedgerState::from_commitments([], 0),
            ledger_config,
        );

        // Empty leader notes.
        let leader = Leader::new(
            genesis_header_id,
            vec![],
            NullifierSecret::zero(),
            ledger_config,
        );

        // Test when no additional blocks are included.
        let recovery_state =
            CryptarchiaConsensusState::<(), (), (), (), ()>::from_cryptarchia_and_unpruned_blocks(
                &Cryptarchia {
                    ledger: ledger_state.clone(),
                    consensus: cryptarchia_engine.clone(),
                },
                &leader,
                HashSet::new(),
            );

        // We configured `k = 2`, and since the canonical chain is 4-block long (blocks
        // `0` to `4`), it means that all forks diverging from and before 2
        // blocks in the past are considered prunable, which are blocks `4` and
        // `5` belonging to the first fork from genesis, block `6` belonging to the
        // second fork from genesis, and block `7` belonging to the fork from block
        // `1`. Block `8` is excluded since it diverged from block `2` which is
        // not yet finalized, as it is only 1 block past, which is less than the
        // configured `k = 2`.
        assert_eq!(
            recovery_state
                .prunable_blocks
                .into_iter()
                .collect::<HashSet<_>>(),
            [
                [4; 32].into(),
                [5; 32].into(),
                [6; 32].into(),
                [7; 32].into()
            ]
            .into()
        );

        // Test when additional blocks are included.
        let recovery_state =
            CryptarchiaConsensusState::<(), (), (), (), ()>::from_cryptarchia_and_unpruned_blocks(
                &Cryptarchia {
                    ledger: ledger_state,
                    consensus: cryptarchia_engine,
                },
                &leader,
                core::iter::once([255; 32].into()).collect(),
            );

        // Result should be the same as above, with the addition of the new block
        assert_eq!(
            recovery_state
                .prunable_blocks
                .into_iter()
                .collect::<HashSet<_>>(),
            [
                [4; 32].into(),
                [5; 32].into(),
                [6; 32].into(),
                [7; 32].into(),
                [255; 32].into()
            ]
            .into()
        );
    }
}
