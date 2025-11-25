use nomos_libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

use crate::config::blend::serde::{
    common::Config as CommonConfig, core::Config as CoreConfig, edge::Config as EdgeConfig,
};

pub mod common;
pub mod core;
pub mod edge;

/// Config object that is part of the global config file.
///
/// This includes all values that are not strictly related to any specific
/// deployment and that users have to specify when starting up the node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub common: CommonConfig,
    pub core: CoreConfig,
    pub edge: EdgeConfig,
}

impl Config {
    pub fn set_listening_address(&mut self, addr: Multiaddr) {
        self.core.backend.listening_address = addr;
    }
}
