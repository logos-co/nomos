use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{balance::Unit, nullifier::NullifierCommitment, NullifierSecret};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Covenant(pub [u8; 32]);

impl Covenant {
    #[must_use]
    pub fn from_vk(covenant_vk: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_CL_COVENANT_COMMIT");
        hasher.update(covenant_vk);
        let covenant_cm: [u8; 32] = hasher.finalize().into();

        Self(covenant_cm)
    }
}

#[must_use]
pub fn derive_unit(unit: &str) -> Unit {
    let mut hasher = Sha256::new();
    hasher.update(b"NOMOS_CL_UNIT");
    hasher.update(unit.as_bytes());
    let unit: Unit = hasher.finalize().into();
    unit
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteCommitment(pub [u8; 32]);

impl NoteCommitment {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct NoteWitness {
    pub value: u64,
    pub unit: Unit,
    pub covenant: Covenant,
    pub state: [u8; 32],
    pub nonce: Nonce,
}

impl NoteWitness {
    #[must_use]
    pub const fn new(
        value: u64,
        unit: Unit,
        covenant: Covenant,
        state: [u8; 32],
        nonce: Nonce,
    ) -> Self {
        Self {
            value,
            unit,
            covenant,
            state,
            nonce,
        }
    }

    pub fn basic(value: u64, unit: Unit, rng: impl RngCore) -> Self {
        let covenant = Covenant([0u8; 32]);
        let nonce = Nonce::random(rng);
        Self::new(value, unit, covenant, [0u8; 32], nonce)
    }

    pub fn stateless(value: u64, unit: Unit, covenant: Covenant, rng: impl RngCore) -> Self {
        Self::new(value, unit, covenant, [0u8; 32], Nonce::random(rng))
    }

    #[must_use]
    pub fn evolved_nonce(&self, nf_sk: NullifierSecret, domain: &[u8]) -> Nonce {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_COIN_EVOLVE");
        hasher.update(domain);
        hasher.update(nf_sk.0);
        hasher.update(self.commit(nf_sk.commit()).0);

        let nonce_bytes: [u8; 32] = hasher.finalize().into();
        Nonce::from_bytes(nonce_bytes)
    }

    #[must_use]
    pub fn commit(&self, nf_pk: NullifierCommitment) -> NoteCommitment {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_CL_NOTE_COMMIT");

        // COMMIT TO BALANCE
        hasher.update(self.value.to_le_bytes());
        hasher.update(self.unit);
        // Important! we don't commit to the balance blinding factor as that may make
        // the notes linkable.

        // COMMIT TO STATE
        hasher.update(self.state);

        // COMMIT TO CONSTRAINT
        hasher.update(self.covenant.0);

        // COMMIT TO NONCE
        hasher.update(self.nonce.as_bytes());

        // COMMIT TO NULLIFIER
        hasher.update(nf_pk.as_bytes());

        let commit_bytes: [u8; 32] = hasher.finalize().into();
        NoteCommitment(commit_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nonce([u8; 32]);

impl Nonce {
    pub fn random(mut rng: impl RngCore) -> Self {
        let mut nonce = [0u8; 32];
        rng.fill_bytes(&mut nonce);
        Self(nonce)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_note_commit_permutations() {
        let (nmo, eth) = (derive_unit("NMO"), derive_unit("ETH"));

        let mut rng = rand::thread_rng();

        let nf_pk = NullifierSecret::random(&mut rng).commit();

        let reference_note = NoteWitness::basic(32, nmo, &mut rng);

        // different notes under same nullifier produce different commitments
        let mutation_tests = [
            NoteWitness {
                value: 12,
                ..reference_note
            },
            NoteWitness {
                unit: eth,
                ..reference_note
            },
            NoteWitness {
                covenant: Covenant::from_vk(&[1u8; 32]),
                ..reference_note
            },
            NoteWitness {
                state: [1u8; 32],
                ..reference_note
            },
            NoteWitness {
                nonce: Nonce::random(&mut rng),
                ..reference_note
            },
        ];

        for n in mutation_tests {
            assert_ne!(n.commit(nf_pk), reference_note.commit(nf_pk));
        }

        // commitment to same note with different nullifiers produce different
        // commitments

        let other_nf_pk = NullifierSecret::random(&mut rng).commit();

        assert_ne!(
            reference_note.commit(nf_pk),
            reference_note.commit(other_nf_pk)
        );
    }
}
