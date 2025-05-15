// The Nullifier is used to detect if a note has
// already been consumed.

// The same nullifier secret may be used across multiple
// notes to allow users to hold fewer secrets. A note
// nonce is used to disambiguate when the same nullifier
// secret is used for multiple notes.

use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::NoteCommitment;

// Maintained privately by note holder
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NullifierSecret(pub [u8; 16]);

// Nullifier commitment is public information that
// can be provided to anyone wishing to transfer
// you a note
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NullifierCommitment([u8; 32]);

// The nullifier attached to input notes to prove an input has not
// already been spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Nullifier([u8; 32]);

impl NullifierSecret {
    pub fn random(mut rng: impl RngCore) -> Self {
        let mut sk = [0u8; 16];
        rng.fill_bytes(&mut sk);
        Self(sk)
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self([0u8; 16])
    }

    #[must_use]
    pub fn commit(&self) -> NullifierCommitment {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_CL_NULL_COMMIT");
        hasher.update(self.0);

        let commit_bytes: [u8; 32] = hasher.finalize().into();
        NullifierCommitment(commit_bytes)
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl NullifierCommitment {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl Nullifier {
    #[must_use]
    pub fn new(sk: NullifierSecret, note_cm: NoteCommitment) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"NOMOS_CL_NULLIFIER");
        hasher.update(sk.0);
        hasher.update(note_cm.0);

        let nf_bytes: [u8; 32] = hasher.finalize().into();
        Self(nf_bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{note::derive_unit, Covenant, Nonce, NoteWitness};

    #[ignore = "nullifier test vectors not stable yet"]
    #[test]
    fn test_nullifier_commitment_vectors() {
        assert_eq!(
            NullifierSecret([0u8; 16]).commit().hex(),
            "384318f9864fe57647bac344e2afdc500a672dedb29d2dc63b004e940e4b382a"
        );
        assert_eq!(
            NullifierSecret([1u8; 16]).commit().hex(),
            "0fd667e6bb39fbdc35d6265726154b839638ea90bcf4e736953ccf27ca5f870b"
        );
        assert_eq!(
            NullifierSecret([u8::MAX; 16]).commit().hex(),
            "1cb78e487eb0b3116389311fdde84cd3f619a4d7f487b29bf5a002eed3784d75"
        );
    }

    #[test]
    fn test_nullifier_same_sk_different_nonce() {
        let mut rng = rand::thread_rng();
        let sk = NullifierSecret::random(&mut rng);
        let note_1 = NoteWitness {
            value: 1,
            unit: derive_unit("NMO"),
            covenant: Covenant::from_vk(&[]),
            state: [0u8; 32],
            nonce: Nonce::random(&mut rng),
        };
        let note_2 = NoteWitness {
            nonce: Nonce::random(&mut rng),
            ..note_1
        };

        let note_cm_1 = note_1.commit(sk.commit());
        let note_cm_2 = note_2.commit(sk.commit());

        let nf_1 = Nullifier::new(sk, note_cm_1);
        let nf_2 = Nullifier::new(sk, note_cm_2);

        assert_ne!(nf_1, nf_2);
    }

    #[test]
    fn test_same_sk_same_nonce_different_note() {
        let mut rng = rand::thread_rng();

        let sk = NullifierSecret::random(&mut rng);
        let nonce = Nonce::random(&mut rng);

        let note_1 = NoteWitness {
            value: 1,
            unit: derive_unit("NMO"),
            covenant: Covenant::from_vk(&[]),
            state: [0u8; 32],
            nonce,
        };

        let note_2 = NoteWitness {
            unit: derive_unit("ETH"),
            ..note_1
        };

        let note_cm_1 = note_1.commit(sk.commit());
        let note_cm_2 = note_2.commit(sk.commit());

        let nf_1 = Nullifier::new(sk, note_cm_1);
        let nf_2 = Nullifier::new(sk, note_cm_2);

        assert_ne!(nf_1, nf_2);
    }
}
