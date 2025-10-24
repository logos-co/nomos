pub mod errors;
pub mod secured_key;

mod ed25519;
mod zk;

use key_management_system_macros::KmsEnumKey;
use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

pub use crate::keys::ed25519::Ed25519Key;
pub use crate::keys::zk::ZkKey;

/// Entity that gathers all keys provided by the KMS crate.
///
/// Works as a [`SecuredKey`] over [`Encoding`], delegating requests to the
/// appropriate key.
#[derive(Serialize, Deserialize, ZeroizeOnDrop, PartialEq, Eq, Clone, KmsEnumKey)]
pub enum Key {
    Ed25519(Ed25519Key),
    Zk(ZkKey),
}
