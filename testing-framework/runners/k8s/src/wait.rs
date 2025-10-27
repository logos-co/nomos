use std::{collections::HashSet, time::Duration};

use k8s_openapi::api::{apps::v1::Deployment, core::v1::Service};
use kube::{Api, Client};
use nomos_http_api_common::paths;
use tokio::time::sleep;

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

pub async fn wait_for_deployment_ready(
    client: &Client,
    namespace: &str,
    name: &str,
    timeout: Duration,
) -> Result<(), String> {
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
            Err(err) => return Err(format!("deployment `{name}` fetch failed: {err}")),
        }

        sleep(interval).await;
        elapsed += interval;
    }

    Err(format!(
        "deployment `{name}` did not become ready within {timeout:?}"
    ))
}

pub async fn find_node_port(
    client: &Client,
    namespace: &str,
    service_name: &str,
    service_port: u16,
) -> Result<u16, String> {
    let interval = Duration::from_secs(1);
    for _ in 0..120 {
        match Api::<Service>::namespaced(client.clone(), namespace)
            .get(service_name)
            .await
        {
            Ok(service) => {
                if let Some(spec) = service.spec
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
            Err(err) => return Err(format!("service `{service_name}` fetch failed: {err}")),
        }
        sleep(interval).await;
    }
    Err(format!(
        "nodeport for service `{service_name}` (port {service_port}) was not allocated in time"
    ))
}

pub async fn wait_for_cluster_ready(
    client: &Client,
    namespace: &str,
    release: &str,
    validator_ports: &[NodeConfigPorts],
    executor_ports: &[NodeConfigPorts],
) -> Result<ClusterPorts, String> {
    if validator_ports.is_empty() {
        return Err("cluster must have at least one validator".into());
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

    wait_for_node_http(
        &validator_allocations
            .iter()
            .map(|ports| ports.api)
            .collect::<Vec<_>>(),
    )
    .await?;

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
        wait_for_node_http(
            &executor_allocations
                .iter()
                .map(|ports| ports.api)
                .collect::<Vec<_>>(),
        )
        .await?;
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

pub async fn wait_for_node_http(ports: &[u16]) -> Result<(), String> {
    if ports.is_empty() {
        return Ok(());
    }

    let client = reqwest::Client::new();
    let mut pending: HashSet<u16> = ports.iter().copied().collect();

    for _ in 0..240 {
        let snapshot: Vec<u16> = pending.iter().copied().collect();
        let mut ready = Vec::new();

        for port in snapshot {
            let url = format!("http://127.0.0.1:{port}{}", paths::CRYPTARCHIA_INFO);
            if let Ok(resp) = client.get(&url).send().await
                && resp.status().is_success()
            {
                ready.push(port);
            }
        }

        for port in ready {
            pending.remove(&port);
        }

        if pending.is_empty() {
            return Ok(());
        }

        sleep(Duration::from_secs(1)).await;
    }

    let remaining: Vec<u16> = pending.into_iter().collect();
    Err(format!(
        "timeout waiting for HTTP endpoints on NodePorts {remaining:?}"
    ))
}

pub async fn wait_for_prometheus_http(port: u16) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/-/ready");

    for _ in 0..240 {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_secs(1)).await;
    }

    Err(format!(
        "timeout waiting for prometheus readiness on NodePort {port}"
    ))
}
