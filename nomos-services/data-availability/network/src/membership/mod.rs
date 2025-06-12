pub mod adapters;
pub mod handler;

use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use nomos_core::block::BlockNumber;
use nomos_da_network_core::SubnetworkId;

pub type Assignations = HashMap<SubnetworkId, HashSet<PeerId>>;
