use serde::{Deserialize, Serialize};

use crate::crypto::{
    keys::{Ed25519PrivateKey, Ed25519PublicKey, KEY_SIZE},
    proofs::{
        quota::{PROOF_OF_QUOTA_SIZE, ProofOfQuota},
        selection::{PROOF_OF_SELECTION_SIZE, ProofOfSelection},
    },
    pseudo_random_sized_bytes, random_sized_bytes,
    signatures::{SIGNATURE_SIZE, Signature},
};

/// A blending header that is fully decapsulated.
/// This must be encapsulated when being sent to the blend network.
#[derive(Clone, Serialize, Deserialize)]
pub struct BlendingHeader {
    pub signing_pubkey: Ed25519PublicKey,
    pub proof_of_quota: ProofOfQuota,
    pub signature: Signature,
    pub proof_of_selection: ProofOfSelection,
    pub is_last: bool,
}

impl BlendingHeader {
    /// Build a blending header with random data based on the provided key.
    /// in the reconstructable way.
    /// Each field in the header is filled with pseudo-random bytes derived from
    /// the key concatenated with a unique byte (1, 2, 3, or 4).
    pub fn pseudo_random(key: &[u8]) -> Self {
        let r1 = pseudo_random_sized_bytes::<KEY_SIZE>(&concat(key, &[1]));
        let r2 = pseudo_random_sized_bytes::<PROOF_OF_QUOTA_SIZE>(&concat(key, &[2]));
        let r3 = pseudo_random_sized_bytes::<SIGNATURE_SIZE>(&concat(key, &[3]));
        let r4 = pseudo_random_sized_bytes::<PROOF_OF_SELECTION_SIZE>(&concat(key, &[4]));
        Self {
            // Unlike the spec, derive a private key from random bytes
            // and then derive the public key from it
            // because a public key cannot always be successfully derived from random bytes.
            // TODO: This will be changed once we have zerocopy serde.
            signing_pubkey: Ed25519PrivateKey::from(r1).public_key(),
            proof_of_quota: ProofOfQuota::from_bytes_unchecked(r2),
            signature: Signature::from(r3),
            proof_of_selection: ProofOfSelection::from_bytes_unchecked(r4),
            is_last: false,
        }
    }

    /// Build a blending header with random data that is not reconstructable.
    pub fn random() -> Self {
        // A random key is used since no reconstruction is needed.
        Self::pseudo_random(&random_sized_bytes::<KEY_SIZE>())
    }
}

fn concat(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().chain(b.iter()).copied().collect::<Vec<_>>()
}
