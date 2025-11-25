use std::path::PathBuf;

use derivative::Derivative;
use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Derivative, Clone)]
#[derivative(Debug)]
pub struct Config {
    /// The non-ephemeral signing key (NSK) corresponding to the public key
    /// registered in the membership (SDP).
    #[serde(with = "nomos_blend_scheduling::serde::ed25519_privkey_hex")]
    #[derivative(Debug = "ignore")]
    pub non_ephemeral_signing_key: Ed25519PrivateKey,
    pub recovery_path_prefix: PathBuf,
}
