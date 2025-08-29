use std::time::Duration;

use nomos_utils::{
    bounded_duration::{MinimalBoundedDuration, SECOND},
    math::PositiveF64,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

const GW_RECV_TIMEOUT_SEC: u64 = 1;
const LEASE_LIFETIME_SECS: u32 = 7200; // 2 hours
const DEFAULT_RENEWAL_DELAY_FRACTION: f64 = 0.8;
const DEFAULT_RETRY_INTERVAL_SEC: u64 = 30;

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
    #[serde(default = "default_renewal_delay_fraction")]
    pub renewal_delay_fraction: PositiveF64,
    #[serde(default = "default_retry_interval")]
    pub retry_interval: Duration,
}

const fn default_timeout() -> Duration {
    Duration::from_secs(GW_RECV_TIMEOUT_SEC)
}

const fn default_lifetime() -> u32 {
    LEASE_LIFETIME_SECS
}

const fn default_max_retries() -> u32 {
    3
}

fn default_renewal_delay_fraction() -> PositiveF64 {
    PositiveF64::try_from(DEFAULT_RENEWAL_DELAY_FRACTION).expect("0.8 is positive")
}

const fn default_retry_interval() -> Duration {
    Duration::from_secs(DEFAULT_RETRY_INTERVAL_SEC)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
            lease_duration: default_lifetime(),
            max_retries: default_max_retries(),
            renewal_delay_fraction: default_renewal_delay_fraction(),
            retry_interval: default_retry_interval(),
        }
    }
}
