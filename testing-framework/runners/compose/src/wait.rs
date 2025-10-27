use std::{collections::HashSet, time::Duration};

use nomos_http_api_common::paths;
use reqwest::Client as ReqwestClient;
use tokio::time::sleep;

pub async fn wait_for_validators(ports: &[u16]) -> Result<(), String> {
    if ports.is_empty() {
        return Err("execution plan must include at least one validator".into());
    }

    let client = ReqwestClient::new();
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
        "timeout waiting for validator HTTP endpoints on ports {remaining:?}"
    ))
}
