pub use poq::CorePathAndSelectors;

pub(crate) type ZkHash = groth16::Fr;
pub(crate) type ZkHasher = poseidon2::Poseidon2Bn254Hasher;

pub mod merkle;
pub mod quota;
pub mod selection;

trait ZkHashExt {
    fn hash(&self) -> ZkHash;
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

trait ZkCompressExt {
    fn compress(&self) -> ZkHash;
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
