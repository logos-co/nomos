use std::{net::SocketAddr, time::Duration};

/// Configuration for the Http Server
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde_with::serde_as]
pub struct AxumBackendSettings {
    /// Socket where the server will be listening on for incoming requests.
    pub address: SocketAddr,
    /// Allowed origins for this server deployment requests.
    pub cors_origins: Vec<String>,
    /// Timeout for API requests in seconds (default: 30)
    #[serde_as(as = "serde_with::DurationSeconds<u64>")]
    #[serde(default = "default_timeout")]
    pub timeout: Duration,
    /// Maximum request body size in bytes (default: 10MB)
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
    /// Maximum number of concurrent requests
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: usize,
    /// Set the interval after which one element of the quota is replenished in
    /// seconds.
    #[serde(default = "default_rate_limit_per_second")]
    pub rate_limit_per_second: u64,
    /// Set quota size that defines how many requests can occur before the
    /// governor middleware starts blocking requests from an IP address and
    /// clients have to wait until the elements of the quota are replenished.
    #[serde(default = "default_rate_limit_burst")]
    pub rate_limit_burst: u32,
}

impl Default for AxumBackendSettings {
    fn default() -> Self {
        Self {
            address: SocketAddr::from(([127, 0, 0, 1], 8080)),
            cors_origins: Vec::new(),
            timeout: default_timeout(),
            max_body_size: default_max_body_size(),
            max_concurrent_requests: default_max_concurrent_requests(),
            rate_limit_per_second: default_rate_limit_per_second(),
            rate_limit_burst: default_rate_limit_burst(),
        }
    }
}

const fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

const fn default_max_body_size() -> usize {
    10 * 1024 * 1024
}

const fn default_max_concurrent_requests() -> usize {
    500
}

const fn default_rate_limit_per_second() -> u64 {
    100
}

const fn default_rate_limit_burst() -> u32 {
    20
}
