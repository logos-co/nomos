use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api, Client,
    api::{ListParams, LogParams},
};
use tracing::{info, warn};

pub async fn dump_namespace_logs(client: &Client, namespace: &str) {
    let pod_names = match list_pod_names(client, namespace).await {
        Ok(names) => names,
        Err(err) => {
            warn!("[k8s-runner] failed to list pods in namespace {namespace}: {err}");
            return;
        }
    };

    for pod_name in pod_names {
        stream_pod_logs(client, namespace, &pod_name).await;
    }
}

async fn list_pod_names(client: &Client, namespace: &str) -> Result<Vec<String>, kube::Error> {
    let list = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default())
        .await?;
    Ok(list
        .into_iter()
        .filter_map(|pod| pod.metadata.name)
        .collect())
}

async fn stream_pod_logs(client: &Client, namespace: &str, pod_name: &str) {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let params = LogParams {
        follow: false,
        tail_lines: Some(500),
        ..Default::default()
    };

    match pods.logs(pod_name, &params).await {
        Ok(log) => info!("[k8s-runner] pod {pod_name} logs:\n{log}"),
        Err(err) => warn!("[k8s-runner] failed to fetch logs for pod {pod_name}: {err}"),
    }
}
