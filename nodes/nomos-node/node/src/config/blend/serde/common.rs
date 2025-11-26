use std::path::PathBuf;

use derivative::Derivative;
use key_management_system_service::keys::Ed25519Key;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Derivative, Clone)]
#[derivative(Debug)]
pub struct Config {
    /// The non-ephemeral signing key (NSK) corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "nomos_blend::message::crypto::serde::ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: Ed25519Key,
    pub recovery_path_prefix: PathBuf,
}
