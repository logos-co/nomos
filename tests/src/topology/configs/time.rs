use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use time::OffsetDateTime;

const DEFAULT_SLOT_TIME: u64 = 2;
const CONSENSUS_SLOT_TIME_VAR: &str = "CONSENSUS_SLOT_TIME";

#[derive(Clone, Debug)]
pub struct GeneralTimeConfig {
    pub slot_duration: Duration,
    pub chain_start_time: OffsetDateTime,
    pub ntp_server: String,
    pub timeout: Duration,
    pub local_socket: SocketAddr,
    pub update_interval: Duration,
}

#[must_use]
pub fn default_time_config() -> GeneralTimeConfig {
    let slot_duration = std::env::var(CONSENSUS_SLOT_TIME_VAR)
        .map(|s| <u64>::from_str(&s).unwrap())
        .unwrap_or(DEFAULT_SLOT_TIME);
    GeneralTimeConfig {
        slot_duration: Duration::from_secs(slot_duration),
        chain_start_time: OffsetDateTime::now_utc(),
        ntp_server: String::from("185.251.115.30"), // 0.europe.pool.ntp.org
        timeout: Duration::from_secs(5),
        local_socket: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 31123),
        update_interval: Duration::from_secs(16),
    }
}
