use std::{fmt, time::Duration};

use futures::future::try_join_all;
use nomos_http_api_common::paths;
use reqwest::Client as ReqwestClient;
use thiserror::Error;
use tokio::time::{sleep, timeout};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeRole {
    Validator,
    Executor,
}

impl NodeRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Validator => "validator",
            Self::Executor => "executor",
        }
    }
}

impl fmt::Display for NodeRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Copy, Debug, Error)]
#[error("timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}")]
pub struct HttpReadinessError {
    role: NodeRole,
    port: u16,
    timeout: Duration,
}

impl HttpReadinessError {
    #[must_use]
    pub const fn new(role: NodeRole, port: u16, timeout: Duration) -> Self {
        Self {
            role,
            port,
            timeout,
        }
    }

    #[must_use]
    pub const fn role(&self) -> NodeRole {
        self.role
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

pub async fn wait_for_http_ports(
    ports: &[u16],
    role: NodeRole,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    wait_for_http_ports_with_host(ports, role, "127.0.0.1", timeout_duration, poll_interval).await
}

pub async fn wait_for_http_ports_with_host(
    ports: &[u16],
    role: NodeRole,
    host: &str,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    if ports.is_empty() {
        return Ok(());
    }

    let client = ReqwestClient::new();
    let probes = ports.iter().copied().map(|port| {
        wait_for_single_port(
            client.clone(),
            port,
            role,
            host,
            timeout_duration,
            poll_interval,
        )
    });

    try_join_all(probes).await.map(|_| ())
}

async fn wait_for_single_port(
    client: ReqwestClient,
    port: u16,
    role: NodeRole,
    host: &str,
    timeout_duration: Duration,
    poll_interval: Duration,
) -> Result<(), HttpReadinessError> {
    let url = format!("http://{host}:{port}{}", paths::CRYPTARCHIA_INFO);
    let probe = async {
        loop {
            let is_ready = client
                .get(&url)
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false);

            if is_ready {
                return;
            }

            sleep(poll_interval).await;
        }
    };

    timeout(timeout_duration, probe)
        .await
        .map_err(|_| HttpReadinessError::new(role, port, timeout_duration))
}
