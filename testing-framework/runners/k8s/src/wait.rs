use std::time::Duration;

use k8s_openapi::api::{apps::v1::Deployment, core::v1::Service};
use kube::{Api, Client, Error as KubeError};
use testing_framework_core::scenario::http_probe::{self, HttpReadinessError, NodeRole};
use thiserror::Error;
use tokio::time::sleep;

use crate::host::node_host;

const DEPLOYMENT_TIMEOUT: Duration = Duration::from_secs(180);
const PROMETHEUS_HTTP_PORT: u16 = 9090;
const PROMETHEUS_SERVICE_NAME: &str = "prometheus";

#[derive(Clone, Copy)]
pub struct NodeConfigPorts {
    pub api: u16,
    pub testing: u16,
}

#[derive(Clone, Copy)]
pub struct NodePortAllocation {
    pub api: u16,
    pub testing: u16,
}

pub struct ClusterPorts {
    pub validators: Vec<NodePortAllocation>,
    pub executors: Vec<NodePortAllocation>,
    pub prometheus: u16,
}

#[derive(Debug, Error)]
pub enum ClusterWaitError {
    #[error("deployment {name} in namespace {namespace} did not become ready within {timeout:?}")]
    DeploymentTimeout {
        name: String,
        namespace: String,
        timeout: Duration,
    },
    #[error("failed to fetch deployment {name}: {source}")]
    DeploymentFetch {
        name: String,
        #[source]
        source: KubeError,
    },
    #[error("failed to fetch service {service}: {source}")]
    ServiceFetch {
        service: String,
        #[source]
        source: KubeError,
    },
    #[error("service {service} did not allocate a node port for {port}")]
    NodePortUnavailable { service: String, port: u16 },
    #[error("cluster must have at least one validator")]
    MissingValidator,
    #[error("timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}")]
    NodeHttpTimeout {
        role: NodeRole,
        port: u16,
        timeout: Duration,
    },
    #[error("timeout waiting for prometheus readiness on NodePort {port}")]
    PrometheusTimeout { port: u16 },
}

pub async fn wait_for_deployment_ready(
    client: &Client,
    namespace: &str,
    name: &str,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    let mut elapsed = Duration::ZERO;
    let interval = Duration::from_secs(2);

    while elapsed <= timeout {
        match Api::<Deployment>::namespaced(client.clone(), namespace)
            .get(name)
            .await
        {
            Ok(deployment) => {
                let desired = deployment
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.replicas)
                    .unwrap_or(1);
                let ready = deployment
                    .status
                    .as_ref()
                    .and_then(|status| status.ready_replicas)
                    .unwrap_or(0);
                if ready >= desired {
                    return Ok(());
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::DeploymentFetch {
                    name: name.to_owned(),
                    source: err,
                });
            }
        }

        sleep(interval).await;
        elapsed += interval;
    }

    Err(ClusterWaitError::DeploymentTimeout {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        timeout,
    })
}

pub async fn find_node_port(
    client: &Client,
    namespace: &str,
    service_name: &str,
    service_port: u16,
) -> Result<u16, ClusterWaitError> {
    let interval = Duration::from_secs(1);
    for _ in 0..120 {
        match Api::<Service>::namespaced(client.clone(), namespace)
            .get(service_name)
            .await
        {
            Ok(service) => {
                if let Some(spec) = service.spec.clone()
                    && let Some(ports) = spec.ports
                {
                    for port in ports {
                        if port.port == i32::from(service_port)
                            && let Some(node_port) = port.node_port
                        {
                            return Ok(node_port as u16);
                        }
                    }
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::ServiceFetch {
                    service: service_name.to_owned(),
                    source: err,
                });
            }
        }
        sleep(interval).await;
    }

    Err(ClusterWaitError::NodePortUnavailable {
        service: service_name.to_owned(),
        port: service_port,
    })
}

pub async fn wait_for_cluster_ready(
    client: &Client,
    namespace: &str,
    release: &str,
    validator_ports: &[NodeConfigPorts],
    executor_ports: &[NodeConfigPorts],
) -> Result<ClusterPorts, ClusterWaitError> {
    if validator_ports.is_empty() {
        return Err(ClusterWaitError::MissingValidator);
    }

    let mut validator_allocations = Vec::with_capacity(validator_ports.len());

    for (index, ports) in validator_ports.iter().enumerate() {
        let name = format!("{release}-validator-{index}");
        wait_for_deployment_ready(client, namespace, &name, DEPLOYMENT_TIMEOUT).await?;
        let api_port = find_node_port(client, namespace, &name, ports.api).await?;
        let testing_port = find_node_port(client, namespace, &name, ports.testing).await?;
        validator_allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
    }

    let validator_api_ports: Vec<u16> = validator_allocations
        .iter()
        .map(|ports| ports.api)
        .collect();
    wait_for_node_http(&validator_api_ports, NodeRole::Validator).await?;

    let mut executor_allocations = Vec::with_capacity(executor_ports.len());
    for (index, ports) in executor_ports.iter().enumerate() {
        let name = format!("{release}-executor-{index}");
        wait_for_deployment_ready(client, namespace, &name, DEPLOYMENT_TIMEOUT).await?;
        let api_port = find_node_port(client, namespace, &name, ports.api).await?;
        let testing_port = find_node_port(client, namespace, &name, ports.testing).await?;
        executor_allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
    }

    if !executor_allocations.is_empty() {
        let executor_api_ports: Vec<u16> =
            executor_allocations.iter().map(|ports| ports.api).collect();
        wait_for_node_http(&executor_api_ports, NodeRole::Executor).await?;
    }

    let prometheus_port = find_node_port(
        client,
        namespace,
        PROMETHEUS_SERVICE_NAME,
        PROMETHEUS_HTTP_PORT,
    )
    .await?;
    wait_for_prometheus_http(prometheus_port).await?;

    Ok(ClusterPorts {
        validators: validator_allocations,
        executors: executor_allocations,
        prometheus: prometheus_port,
    })
}

async fn wait_for_node_http(ports: &[u16], role: NodeRole) -> Result<(), ClusterWaitError> {
    let host = node_host();
    http_probe::wait_for_http_ports_with_host(
        ports,
        role,
        &host,
        Duration::from_secs(240),
        Duration::from_secs(1),
    )
    .await
    .map_err(map_http_error)
}

const fn map_http_error(error: HttpReadinessError) -> ClusterWaitError {
    ClusterWaitError::NodeHttpTimeout {
        role: error.role(),
        port: error.port(),
        timeout: error.timeout(),
    }
}

pub async fn wait_for_prometheus_http(port: u16) -> Result<(), ClusterWaitError> {
    let client = reqwest::Client::new();
    let url = format!("http://{}:{port}/-/ready", node_host());

    for _ in 0..240 {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_secs(1)).await;
    }

    Err(ClusterWaitError::PrometheusTimeout { port })
}
