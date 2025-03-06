use std::{str::FromStr as _, time::Duration};

use time::OffsetDateTime;

const DEFAULT_SLOT_TIME: u64 = 2;
const CONSENSUS_SLOT_TIME_VAR: &str = "CONSENSUS_SLOT_TIME";

#[derive(Clone, Debug)]
pub struct GeneralTimeConfig {
    pub slot_duration: Duration,
    pub chain_start_time: OffsetDateTime,
}

#[must_use]
pub fn default_time_config() -> GeneralTimeConfig {
    let slot_duration = std::env::var(CONSENSUS_SLOT_TIME_VAR)
        .map(|s| <u64>::from_str(&s).unwrap())
        .unwrap_or(DEFAULT_SLOT_TIME);
    GeneralTimeConfig {
        slot_duration: Duration::from_secs(slot_duration),
        chain_start_time: OffsetDateTime::now_utc(),
    }
}
