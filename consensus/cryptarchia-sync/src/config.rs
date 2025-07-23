use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSeconds};

pub(crate) const DEFAULT_RESPONSE_TIMEOUT_MS: u64 = 5000;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// The maximum number of ms to wait for a peer to respond
    /// with a message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<DurationMilliSeconds<u64>>")]
    pub peer_response_timeout_ms: Option<Duration>,
}
