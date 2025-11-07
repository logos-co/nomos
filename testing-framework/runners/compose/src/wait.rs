use std::time::Duration;

use testing_framework_core::{
    adjust_timeout,
    scenario::http_probe::{self, HttpReadinessError, NodeRole},
};

const DEFAULT_WAIT: Duration = Duration::from_secs(90);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub async fn wait_for_validators(ports: &[u16]) -> Result<(), HttpReadinessError> {
    wait_for_ports(ports, NodeRole::Validator).await
}

pub async fn wait_for_executors(ports: &[u16]) -> Result<(), HttpReadinessError> {
    wait_for_ports(ports, NodeRole::Executor).await
}

async fn wait_for_ports(ports: &[u16], role: NodeRole) -> Result<(), HttpReadinessError> {
    http_probe::wait_for_http_ports(ports, role, adjust_timeout(DEFAULT_WAIT), POLL_INTERVAL).await
}
