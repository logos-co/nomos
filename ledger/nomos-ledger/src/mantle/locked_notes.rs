use std::collections::HashSet;

use nomos_core::{
    mantle::NoteId,
    sdp::{MinStake, ServiceType},
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::UtxoTree;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Note does not exist: {0:?}")]
    NoteDoesNotExist(NoteId),
    #[error("Note {note_id:?} insufficient value: {value}")]
    NoteInsufficientValue { note_id: NoteId, value: u64 },
    #[error("Note {note_id:?} already used for service {service_type:?}")]
    NoteAlreadyUsedForService {
        note_id: NoteId,
        service_type: ServiceType,
    },
    #[error("Note {note_id:?} not locked for {service_type:?}")]
    NoteNotLockedForService {
        note_id: NoteId,
        service_type: ServiceType,
    },
    #[error("Note is not locked: {0:?}")]
    NoteNotLocked(NoteId),
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LockedNotes {
    locked_notes: rpds::HashTrieMapSync<NoteId, HashSet<ServiceType>>,
}

impl LockedNotes {
    #[must_use]
    pub fn new() -> Self {
        Self {
            locked_notes: rpds::HashTrieMapSync::new_sync(),
        }
    }

    #[must_use]
    pub fn contains(&self, id: &NoteId) -> bool {
        self.locked_notes.contains_key(id)
    }

    pub fn lock(
        mut self,
        utxo_tree: &UtxoTree,
        min_stake: &MinStake,
        service_type: ServiceType,
        note_id: &NoteId,
    ) -> Result<Self, Error> {
        let Some((utxo, _)) = utxo_tree.utxos().get(note_id) else {
            return Err(Error::NoteDoesNotExist(*note_id));
        };

        if utxo.note.value < min_stake.threshold {
            return Err(Error::NoteInsufficientValue {
                note_id: *note_id,
                value: utxo.note.value,
            });
        }

        if let Some(services) = self.locked_notes.get_mut(note_id) {
            if services.contains(&service_type) {
                return Err(Error::NoteAlreadyUsedForService {
                    note_id: *note_id,
                    service_type,
                });
            }
            services.insert(service_type);
        } else {
            let services = [service_type].into();
            self.locked_notes = self.locked_notes.insert(*note_id, services);
        }

        Ok(self)
    }

    pub fn unlock(mut self, service_type: ServiceType, note_id: &NoteId) -> Result<Self, Error> {
        if let Some(services) = self.locked_notes.get(note_id) {
            if !services.contains(&service_type) {
                return Err(Error::NoteNotLockedForService {
                    note_id: *note_id,
                    service_type,
                });
            }

            let mut updated_services = services.clone();
            updated_services.remove(&service_type);

            if updated_services.is_empty() {
                self.locked_notes = self.locked_notes.remove(note_id);
            } else {
                self.locked_notes = self.locked_notes.insert(*note_id, updated_services);
            }

            Ok(self)
        } else {
            Err(Error::NoteNotLocked(*note_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nomos_core::sdp::{MinStake, ServiceType};

    use crate::{
        cryptarchia::tests::{genesis_state, utxo},
        mantle::locked_notes::{Error, LockedNotes},
    };

    #[test]
    fn test_lock_success() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let locked_notes = LockedNotes::new();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };

        let locked_notes_bn = locked_notes
            .lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id)
            .expect("Should be able to lock for BN service");

        assert!(locked_notes_bn.contains(&note_id));
        assert_eq!(
            locked_notes_bn.locked_notes.get(&note_id),
            Some(&HashSet::from([ServiceType::BlendNetwork]))
        );

        let locked_notes_both = locked_notes_bn
            .lock(
                utxo_tree,
                &min_stake,
                ServiceType::DataAvailability,
                &note_id,
            )
            .expect("Should be able to lock for DA service");

        assert!(locked_notes_both.contains(&note_id));
        assert_eq!(
            locked_notes_both.locked_notes.get(&note_id),
            Some(&HashSet::from([
                ServiceType::BlendNetwork,
                ServiceType::DataAvailability
            ]))
        );
    }

    #[test]
    fn test_lock_fail_already_used() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let locked_notes = LockedNotes::new();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };

        let locked_notes_once = locked_notes
            .lock(
                utxo_tree,
                &min_stake,
                ServiceType::DataAvailability,
                &note_id,
            )
            .unwrap();

        let result = locked_notes_once.lock(
            utxo_tree,
            &min_stake,
            ServiceType::DataAvailability,
            &note_id,
        );

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            Error::NoteAlreadyUsedForService {
                note_id,
                service_type: ServiceType::DataAvailability
            }
        );
    }

    #[test]
    fn lock_fail_insufficient() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let locked_notes = LockedNotes::new();

        let min_stake = MinStake {
            threshold: 999_999,
            timestamp: 0,
        };

        let result = locked_notes.lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            Error::NoteInsufficientValue {
                note_id,
                value: 10000
            }
        );
    }

    #[test]
    fn test_lock_fail_does_not_exist() {
        let note_id = utxo().id();
        let state = genesis_state(&[utxo()]);
        let utxo_tree = state.latest_commitments();
        let locked_notes = LockedNotes::new();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };

        let result = locked_notes.lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::NoteDoesNotExist(note_id));
    }
    #[test]
    fn test_unlock_one_of_two_services() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };
        let locked_for_both = LockedNotes::new()
            .lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id)
            .unwrap()
            .lock(
                utxo_tree,
                &min_stake,
                ServiceType::DataAvailability,
                &note_id,
            )
            .unwrap();

        let locked_for_da_only = locked_for_both
            .unlock(ServiceType::BlendNetwork, &note_id)
            .expect("Should unlock BN service");

        assert!(locked_for_da_only.contains(&note_id));
        assert_eq!(
            locked_for_da_only.locked_notes.get(&note_id),
            Some(&HashSet::from([ServiceType::DataAvailability]))
        );
    }

    #[test]
    fn test_unlock_last_service_removes_note() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };
        let locked_for_bn = LockedNotes::new()
            .lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id)
            .unwrap();

        let fully_unlocked = locked_for_bn
            .unlock(ServiceType::BlendNetwork, &note_id)
            .expect("Should unlock the last service");

        assert!(!fully_unlocked.contains(&note_id));
        assert!(fully_unlocked.locked_notes.is_empty());
    }

    #[test]
    fn test_unlock_note_not_locked() {
        let note_id = utxo().id();
        let empty_notes = LockedNotes::new();

        let result = empty_notes.unlock(ServiceType::BlendNetwork, &note_id);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::NoteNotLocked(note_id));
    }

    #[test]
    fn test_unlock_fail_if_not_locked_for_specific_service() {
        let utxo = utxo();
        let note_id = utxo.id();
        let state = genesis_state(&[utxo]);
        let utxo_tree = state.latest_commitments();
        let min_stake = MinStake {
            threshold: 1,
            timestamp: 0,
        };
        let locked_for_bn = LockedNotes::new()
            .lock(utxo_tree, &min_stake, ServiceType::BlendNetwork, &note_id)
            .unwrap();

        let result = locked_for_bn.unlock(ServiceType::DataAvailability, &note_id);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            Error::NoteNotLockedForService {
                note_id,
                service_type: ServiceType::DataAvailability
            }
        );
    }
}
