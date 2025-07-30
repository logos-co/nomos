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
    /// The max number of blocks to download by a single request.
    // TODO: This is not used anywhere yet. Add this to the download request
    // by modifying related implemn
    pub download_size: NonZeroUsize,
    /// The max number of downloads to repeat for a single peer.
    pub max_download_iterations: NonZeroUsize,
}
