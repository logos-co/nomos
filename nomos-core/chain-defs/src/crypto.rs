use blake2::digest::typenum::U32;
pub type Hasher = blake2::Blake2b<U32>;
pub use blake2::digest::Digest;
