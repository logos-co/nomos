use nomos_core::da::{attestation, Signer};
use serde::{Deserialize, Serialize};

use crate::{hash, Voter};

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Attestation {
    blob_hash: [u8; 32],
    attester: Voter,
    sig: Vec<u8>,
}

impl Attestation {
    pub fn new_signed<S: Signer>(blob_hash: [u8; 32], attester: Voter, key_pair: &S) -> Self {
        let attestation_hash = hash([blob_hash, attester].concat());
        let sig = key_pair.sign(&attestation_hash);
        Self {
            blob_hash,
            attester,
            sig,
        }
    }
}

impl attestation::Attestation for Attestation {
    type Hash = [u8; 32];

    fn blob_hash(&self) -> Self::Hash {
        self.blob_hash
    }

    fn hash(&self) -> Self::Hash {
        hash([self.blob_hash, self.attester].concat())
    }

    fn signature(&self) -> &[u8] {
        self.sig.as_ref()
    }
}
