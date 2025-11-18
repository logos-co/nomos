use nomos_libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

use crate::config::blend::serde::{
    common::Settings as CommonSettings, core::Settings as CoreSettings,
    edge::Settings as EdgeSettings,
};

pub mod common;
pub mod core;
pub mod edge;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(flatten)]
    pub common: CommonSettings,
    pub core: CoreSettings,
    pub edge: EdgeSettings,
}

impl Config {
    pub fn set_listening_address(&mut self, _addr: Multiaddr) {
        unimplemented!()
    }

    pub fn set_blend_layers(&mut self, _layers: u64) {
        unimplemented!()
    }
}
