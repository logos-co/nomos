use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr as _,
    time::Duration,
};

use time::OffsetDateTime;

const DEFAULT_SLOT_TIME: u64 = 2;
const CONSENSUS_SLOT_TIME_VAR: &str = "CONSENSUS_SLOT_TIME";
const DEFAULT_CHAIN_START_OFFSET_SECS: u64 = 60;

#[derive(Clone, Debug)]
pub struct GeneralTimeConfig {
    pub slot_duration: Duration,
    pub chain_start_time: OffsetDateTime,
    pub ntp_server: String,
    pub timeout: Duration,
    pub interface: IpAddr,
    pub update_interval: Duration,
}

#[must_use]
pub fn default_time_config() -> GeneralTimeConfig {
    let slot_duration = std::env::var(CONSENSUS_SLOT_TIME_VAR)
        .map(|s| <u64>::from_str(&s).unwrap())
        .unwrap_or(DEFAULT_SLOT_TIME);
    GeneralTimeConfig {
        slot_duration: Duration::from_secs(slot_duration),
        chain_start_time: OffsetDateTime::now_utc()
            - Duration::from_secs(DEFAULT_CHAIN_START_OFFSET_SECS),
        ntp_server: String::from("pool.ntp.org:123"),
        timeout: Duration::from_secs(5),
        interface: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        update_interval: Duration::from_secs(16),
    }
}
