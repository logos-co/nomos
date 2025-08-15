use std::time::Duration;

use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

const NATPMP_RECV_TIMEOUT_SEC: u64 = 1;
const NATPMP_LIFETIME_SECS: u32 = 7200; // 2 hours

#[serde_as]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default = "default_timeout")]
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub timeout: Duration,
    #[serde(default = "default_lifetime")]
    pub lease_duration: u32,
}

const fn default_timeout() -> Duration {
    Duration::from_secs(NATPMP_RECV_TIMEOUT_SEC)
}

const fn default_lifetime() -> u32 {
    NATPMP_LIFETIME_SECS
}
