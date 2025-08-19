use std::time::Duration;

use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

const NATPMP_RECV_TIMEOUT_SEC: u64 = 1;
const NATPMP_LIFETIME_SECS: u32 = 7200; // 2 hours

#[serde_as]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_timeout")]
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub timeout: Duration,
    #[serde(default = "default_lifetime")]
    pub lease_duration: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

const fn default_timeout() -> Duration {
    Duration::from_secs(NATPMP_RECV_TIMEOUT_SEC)
}

const fn default_lifetime() -> u32 {
    NATPMP_LIFETIME_SECS
}

const fn default_max_retries() -> u32 {
    3 // Default to 3 retry attempts
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            lease_duration: default_lifetime(),
            max_retries: default_max_retries(),
        }
    }
}
