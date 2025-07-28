use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

const MAX_ORPHAN_CACHE_SIZE: NonZeroUsize =
    NonZeroUsize::new(5).expect("MAX_ORPHAN_CACHE_SIZE must be non-zero");

const MAX_NUMBER_OF_ITERATIONS: NonZeroUsize =
    NonZeroUsize::new(10).expect("MAX_NUMBER_OF_ITERATIONS must be non-zero");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrphanConfig {
    /// The maximum number of pending orphans to keep in the cache.
    #[serde(default = "default_max_orphan_cache_size")]
    pub max_orphan_cache_size: NonZeroUsize,
    /// Maximum number of download requests per orphan
    #[serde(default = "default_max_number_of_requests")]
    pub max_number_of_requests: NonZeroUsize,
}

const fn default_max_orphan_cache_size() -> NonZeroUsize {
    MAX_ORPHAN_CACHE_SIZE
}

const fn default_max_number_of_requests() -> NonZeroUsize {
    MAX_NUMBER_OF_ITERATIONS
}
