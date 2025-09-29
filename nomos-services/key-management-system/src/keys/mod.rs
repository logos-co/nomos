pub mod errors;
pub mod secured_key;

#[cfg(any(feature = "key-ed25519", test))]
mod ed25519;
#[cfg(feature = "key-zk")]
mod zk;

use key_management_system_macros::KmsEnumKey;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

#[cfg(any(feature = "key-ed25519", test))]
pub use crate::keys::ed25519::Ed25519Key;
#[cfg(feature = "key-zk")]
pub use crate::keys::zk::ZkKey;

/// Entity that gathers all keys provided by the KMS crate.
///
/// Works as a [`SecuredKey`] over [`Encoding`], delegating requests to the
/// appropriate key.
#[expect(dead_code, reason = "Will be used when integrating the KMS service.")]
#[derive(Serialize, Deserialize, ZeroizeOnDrop, KmsEnumKey)]
pub enum Key {
    #[cfg(any(feature = "key-ed25519", test))]
    Ed25519(Ed25519Key),
    #[cfg(feature = "key-zk")]
    Zk(ZkKey),
}
