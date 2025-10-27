pub mod nodes;
pub mod scenario;
pub mod topology;

use std::{env, ops::Mul as _, sync::LazyLock, time::Duration};

pub use integration_configs::{
    IS_DEBUG_TRACING, node_address_from_port, secret_key_to_peer_id, secret_key_to_provider_id,
    topology::configs::da::GLOBAL_PARAMS_PATH,
};

static IS_SLOW_TEST_ENV: LazyLock<bool> =
    LazyLock::new(|| env::var("SLOW_TEST_ENV").is_ok_and(|s| s == "true"));

/// In slow test environments like Codecov, use 2x timeout.
#[must_use]
pub fn adjust_timeout(d: Duration) -> Duration {
    if *IS_SLOW_TEST_ENV { d.mul(2) } else { d }
}
