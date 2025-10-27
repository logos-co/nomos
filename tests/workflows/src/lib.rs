use std::net::TcpListener;

use testing_framework_core::scenario::Metrics;

pub mod workloads;

const PROM_URL_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_URL";

/// Configure Prometheus metrics for runner tests, lazily binding an ephemeral
/// port when necessary.
#[must_use]
pub fn configure_prometheus_metrics() -> Metrics {
    let url = std::env::var(PROM_URL_ENV).unwrap_or_else(|_| allocate_prometheus_endpoint());

    Metrics::from_prometheus_str(&url).unwrap_or_else(|err| {
        panic!("failed to configure prometheus metrics using `{PROM_URL_ENV}` ({url}): {err}")
    })
}

fn allocate_prometheus_endpoint() -> String {
    let (url, _) = allocate_ephemeral_prometheus();
    url
}

fn allocate_ephemeral_prometheus() -> (String, u16) {
    let listener =
        TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port for prometheus metrics");
    let port = listener
        .local_addr()
        .expect("ephemeral prometheus port")
        .port();
    drop(listener);

    (format!("http://127.0.0.1:{port}"), port)
}
