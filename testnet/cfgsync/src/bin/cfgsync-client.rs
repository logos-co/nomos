use std::{env, fs, net::Ipv4Addr, process, time::Duration};

use cfgsync::{client::get_config, server::ClientIp};
use nomos_executor::config::Config as ExecutorConfig;
use nomos_node::Config as ValidatorConfig;
use serde::{Serialize, de::DeserializeOwned};
use tokio::time::sleep;

fn parse_ip(ip_str: &str) -> Ipv4Addr {
    ip_str.parse().unwrap_or_else(|_| {
        eprintln!("Invalid IP format, defaulting to 127.0.0.1");
        Ipv4Addr::LOCALHOST
    })
}

async fn pull_to_file<Config: Serialize + DeserializeOwned>(
    ip: Ipv4Addr,
    identifier: String,
    url: &str,
    config_file: &str,
) -> Result<(), String> {
    let config = get_config::<Config>(
        ClientIp {
            ip,
            identifier,
            network_port: env::var("CFG_NETWORK_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
            da_network_port: env::var("CFG_DA_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
            blend_port: env::var("CFG_BLEND_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
            api_port: env::var("CFG_API_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
            testing_http_port: env::var("CFG_TESTING_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
        },
        url,
    )
    .await?;
    let yaml = serde_yaml::to_string(&config)
        .map_err(|err| format!("Failed to serialize config to YAML: {err}"))?;

    fs::write(config_file, yaml).map_err(|err| format!("Failed to write config to file: {err}"))?;

    println!("Config saved to {config_file}");
    Ok(())
}

#[tokio::main]
async fn main() {
    let config_file_path = env::var("CFG_FILE_PATH").unwrap_or_else(|_| "config.yaml".to_owned());
    let server_addr =
        env::var("CFG_SERVER_ADDR").unwrap_or_else(|_| "http://127.0.0.1:4400".to_owned());
    let ip = parse_ip(&env::var("CFG_HOST_IP").unwrap_or_else(|_| "127.0.0.1".to_owned()));
    let identifier =
        env::var("CFG_HOST_IDENTIFIER").unwrap_or_else(|_| "unidentified-node".to_owned());

    let host_kind = env::var("CFG_HOST_KIND").unwrap_or_else(|_| "validator".to_owned());

    let node_config_endpoint = match host_kind.as_str() {
        "executor" => format!("{server_addr}/executor"),
        _ => format!("{server_addr}/validator"),
    };

    let max_retries = env::var("CFG_CLIENT_RETRIES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(10);
    let retry_delay = env::var("CFG_CLIENT_RETRY_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or_else(|| Duration::from_secs(1), Duration::from_millis);

    let mut last_error = None;

    for attempt in 0..=max_retries {
        let result = match host_kind.as_str() {
            "executor" => {
                pull_to_file::<ExecutorConfig>(
                    ip,
                    identifier.clone(),
                    &node_config_endpoint,
                    &config_file_path,
                )
                .await
            }
            _ => {
                pull_to_file::<ValidatorConfig>(
                    ip,
                    identifier.clone(),
                    &node_config_endpoint,
                    &config_file_path,
                )
                .await
            }
        };

        match result {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(err) => {
                eprintln!(
                    "cfgsync-client attempt {}/{} failed: {err}",
                    attempt + 1,
                    max_retries + 1
                );
                last_error = Some(err);
                if attempt < max_retries {
                    sleep(retry_delay).await;
                }
            }
        }
    }

    // Handle error if the config request fails
    if let Some(err) = last_error {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}
