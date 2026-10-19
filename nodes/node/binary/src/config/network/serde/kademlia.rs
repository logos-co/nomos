use core::{num::NonZeroUsize, time::Duration};

use lb_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// The timeout for a single query
    #[serde_as(as = "Option<MinimalBoundedDuration<1, SECOND>>")]
    pub query_timeout: Option<Duration>,

    /// The replication factor to use
    pub replication_factor: Option<NonZeroUsize>,

    /// The allowed level of parallelism for iterative queries
    pub parallelism: Option<NonZeroUsize>,

    /// Require iterative queries to use disjoint paths
    pub disjoint_query_paths: Option<bool>,

    /// Maximum allowed size of individual Kademlia packets
    pub max_packet_size: Option<usize>,

    /// The k-bucket insertion strategy
    pub kbucket_inserts: Option<KBucketInserts>,

    /// The caching strategy
    pub caching: Option<CachingSettings>,

    /// The interval for periodic bootstrap
    /// If enabled the periodic bootstrap will run at this interval in addition
    /// to the automatic bootstrap that is triggered when a new peer is added
    /// `0s` disables periodic bootstrap
    #[serde_as(as = "Option<MinimalBoundedDuration<0, SECOND>>")]
    pub periodic_bootstrap_interval: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KBucketInserts {
    OnConnected,
    Manual,
}

impl From<KBucketInserts> for lb_libp2p::config::KBucketInserts {
    fn from(value: KBucketInserts) -> Self {
        match value {
            KBucketInserts::OnConnected => Self::OnConnected,
            KBucketInserts::Manual => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "config")]
pub enum CachingSettings {
    Disabled,
    Enabled { max_peers: u16 },
}

impl From<CachingSettings> for lb_libp2p::config::CachingSettings {
    fn from(value: CachingSettings) -> Self {
        match value {
            CachingSettings::Disabled => Self::Disabled,
            CachingSettings::Enabled { max_peers } => Self::Enabled { max_peers },
        }
    }
}
