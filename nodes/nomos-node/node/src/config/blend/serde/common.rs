use std::path::PathBuf;

use derivative::Derivative;
use key_management_system_service::keys::UnsecuredEd25519Key;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Derivative, Clone)]
#[derivative(Debug)]
pub struct Config {
    /// The non-ephemeral signing key (NSK) corresponding to the public key
    /// registered in the membership (SDP).
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: UnsecuredEd25519Key,
    pub recovery_path_prefix: PathBuf,
}
