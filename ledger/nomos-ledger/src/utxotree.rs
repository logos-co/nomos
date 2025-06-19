use nomos_core::mantle::{
    merkle::{self, PathNode},
    Note, NoteId, Utxo,
};
// up to 2^14 commitments
const MAX_UTXOS: usize = 1 << 14;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UtxoTree {
    utxos: rpds::RedBlackTreeMapSync<NoteId, Note>,
}

// TODO: change to sparse computation
fn note_id_leaves(utxos: &rpds::RedBlackTreeMapSync<NoteId, Note>) -> [[u8; 32]; MAX_UTXOS] {
    let note_id_bytes: Vec<Vec<u8>> = utxos
        .iter()
        .map(|(id, _utxo)| id.as_bytes().to_vec())
        .collect();
    debug_assert!(note_id_bytes.is_sorted());
    merkle::padded_leaves::<MAX_UTXOS>(&note_id_bytes)
}

impl UtxoTree {
    #[must_use]
    pub fn insert(&self, utxo: Utxo) -> Self {
        assert!(self.utxos.size() < MAX_UTXOS, "UtxoTree is full");
        let utxos = self.utxos.insert(utxo.id(), utxo.note);
        Self { utxos }
    }

    #[must_use]
    pub fn remove(&self, utxo: &Utxo) -> Self {
        let utxos = self.utxos.remove(&utxo.id());
        Self { utxos }
    }

    // TODO: cache if called frequently
    #[must_use]
    pub fn root(&self) -> [u8; 32] {
        merkle::root(note_id_leaves(&self.utxos))
    }

    #[must_use]
    pub fn witness(&self, id: &NoteId) -> Option<Vec<PathNode>> {
        let leaves = note_id_leaves(&self.utxos);
        leaves
            .binary_search(&id.0)
            .ok()
            .map(|idx| merkle::path(leaves, idx))
    }

    #[must_use]
    pub const fn utxos(&self) -> &rpds::RedBlackTreeMapSync<NoteId, Note> {
        &self.utxos
    }

    #[must_use]
    pub fn contains(&self, utxo: &Utxo) -> bool {
        self.utxos.contains_key(&utxo.id())
    }
}

impl FromIterator<Utxo> for UtxoTree {
    fn from_iter<I: IntoIterator<Item = Utxo>>(iter: I) -> Self {
        let utxos = iter
            .into_iter()
            .map(|utxo| {
                let note = Note::new(utxo.note.value, utxo.note.pk);
                (utxo.id(), note)
            })
            .collect::<rpds::RedBlackTreeMapSync<NoteId, Note>>();
        Self { utxos }
    }
}
