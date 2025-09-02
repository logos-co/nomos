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
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedNotes {
    // Locked note can be associated with BN and DA services only.
    // Array index 0 of `[bool; 2]` represents BN service, index 1 represents DA service.
    locked_notes: rpds::HashTrieMapSync<NoteId, [bool; 2]>,
}

impl LockedNotes {
    pub fn new() -> Self {
        Self {
            locked_notes: rpds::HashTrieMapSync::new_sync(),
        }
    }

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
        let Some((note, _)) = utxo_tree.utxos().get(note_id) else {
            return Err(Error::NoteDoesNotExist(*note_id));
        };

        if note.value < min_stake.threshold {
            return Err(Error::NoteInsufficientValue {
                note_id: *note_id,
                value: note.value,
            });
        }

        let idx = match service_type {
            ServiceType::BlendNetwork => 0,
            ServiceType::DataAvailability => 1,
        };

        if let Some(services) = self.locked_notes.get_mut(note_id) {
            if services[idx] {
                return Err(Error::NoteAlreadyUsedForService {
                    note_id: *note_id,
                    service_type,
                });
            }
            services[idx] = true;
        } else {
            let mut services = [false, false];
            services[idx] = true;
            self.locked_notes = self.locked_notes.insert(*note_id, services);
        }

        Ok(self)
    }
}
