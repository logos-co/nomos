use std::{collections::HashSet, hash::Hash, num::NonZeroUsize, time::Duration};

use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootstrapConfig<NodeId>
where
    NodeId: Clone + Eq + Hash,
{
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub prolonged_bootstrap_period: Duration,
    pub force_bootstrap: bool,
    pub ibd: IbdConfig<NodeId>,
}

/// IBD configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IbdConfig<NodeId>
where
    NodeId: Clone + Eq + Hash,
{
    /// Peers to download blocks from.
    pub peers: HashSet<NodeId>,
    /// The max number of downloads to repeat for a single peer.
    #[serde(default = "default_max_download_iterations")]
    pub max_download_iterations: NonZeroUsize,
}

const fn default_max_download_iterations() -> NonZeroUsize {
    NonZeroUsize::new(1000).unwrap()
}
