use nomos_core::crypto::{ZkHash, ZkHasher};
use thiserror::Error;

use crate::quota::inputs::prove::public::{CoreInputs, LeaderInputs};

pub mod quota;
pub mod selection;

/// The inputs required to verify a Proof of Quota, without the signing key,
/// which is retrieved from the public header of the message layer being
/// verified.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
pub struct PoQVerificationInputsMinusSigningKey {
    pub session: u64,
    pub core: CoreInputs,
    pub leader: LeaderInputs,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid Proof of Quota: {0}.")]
    ProofOfQuota(#[from] quota::Error),
    #[error("Invalid Proof of Selection: {0}.")]
    ProofOfSelection(selection::Error),
}

trait ZkHashExt {
    fn hash(&self) -> ZkHash;
}

trait ZkCompressExt {
    fn compress(&self) -> ZkHash;
}

impl<T> ZkHashExt for T
where
    T: AsRef<[ZkHash]>,
{
    fn hash(&self) -> ZkHash {
        let mut hasher = ZkHasher::new();
        hasher.update(self.as_ref());
        hasher.finalize()
    }
}

impl ZkCompressExt for [ZkHash; 2] {
    fn compress(&self) -> ZkHash {
        let mut hasher = ZkHasher::new();
        hasher.compress(self);
        hasher.finalize()
    }
}

impl ZkCompressExt for &[ZkHash; 2] {
    fn compress(&self) -> ZkHash {
        let mut hasher = ZkHasher::new();
        hasher.compress(self);
        hasher.finalize()
    }
}
