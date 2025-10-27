use std::{collections::HashMap, sync::Arc};

use prometheus_http_query::{Client as PrometheusClient, response::Data as PrometheusData};
use reqwest::Url;
use tracing::warn;

pub const CONSENSUS_PROCESSED_BLOCKS: &str = "consensus_processed_blocks";
pub const CONSENSUS_TRANSACTIONS_TOTAL: &str = "consensus_transactions_total";
const CONSENSUS_TRANSACTIONS_VALIDATOR_QUERY: &str =
    r#"sum(consensus_transactions_total{job=~"validator-.*"})"#;

#[derive(Clone, Default)]
pub struct Metrics {
    prometheus: Option<Arc<PrometheusEndpoint>>,
}

impl Metrics {
    #[must_use]
    pub const fn empty() -> Self {
        Self { prometheus: None }
    }

    pub fn from_prometheus(url: Url) -> Result<Self, MetricsError> {
        let handle = Arc::new(PrometheusEndpoint::new(url)?);
        Ok(Self::empty().with_prometheus_endpoint(handle))
    }

    pub fn from_prometheus_str(raw_url: &str) -> Result<Self, MetricsError> {
        Url::parse(raw_url)
            .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))
            .and_then(Self::from_prometheus)
    }

    #[must_use]
    pub fn with_prometheus_endpoint(mut self, handle: Arc<PrometheusEndpoint>) -> Self {
        self.prometheus = Some(handle);
        self
    }

    #[must_use]
    pub fn prometheus(&self) -> Option<Arc<PrometheusEndpoint>> {
        self.prometheus.as_ref().map(Arc::clone)
    }

    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.prometheus.is_some()
    }

    pub fn instant_values(&self, query: &str) -> Result<Vec<f64>, MetricsError> {
        let handle = self
            .prometheus()
            .ok_or_else(|| MetricsError::new("prometheus endpoint unavailable"))?;
        handle.instant_values(query)
    }

    pub fn counter_value(&self, query: &str) -> Result<f64, MetricsError> {
        let handle = self
            .prometheus()
            .ok_or_else(|| MetricsError::new("prometheus endpoint unavailable"))?;
        handle.counter_value(query)
    }

    pub fn consensus_processed_blocks(&self) -> Result<f64, MetricsError> {
        self.counter_value(CONSENSUS_PROCESSED_BLOCKS)
    }

    pub fn consensus_transactions_total(&self) -> Result<f64, MetricsError> {
        let handle = self
            .prometheus()
            .ok_or_else(|| MetricsError::new("prometheus endpoint unavailable"))?;

        match handle.instant_samples(CONSENSUS_TRANSACTIONS_VALIDATOR_QUERY) {
            Ok(samples) if !samples.is_empty() => {
                return Ok(samples.into_iter().map(|sample| sample.value).sum());
            }
            Ok(_) => {
                warn!(
                    query = CONSENSUS_TRANSACTIONS_VALIDATOR_QUERY,
                    "validator-specific consensus transaction metric returned no samples; falling back to aggregate counter"
                );
            }
            Err(err) => {
                warn!(
                    query = CONSENSUS_TRANSACTIONS_VALIDATOR_QUERY,
                    error = %err,
                    "failed to query validator-specific consensus transaction metric; falling back to aggregate counter"
                );
            }
        }

        handle.counter_value(CONSENSUS_TRANSACTIONS_TOTAL)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("{0}")]
    Store(String),
}

impl MetricsError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }
}

pub struct PrometheusEndpoint {
    base_url: Url,
    client: PrometheusClient,
}

#[derive(Clone, Debug)]
pub struct PrometheusInstantSample {
    pub labels: HashMap<String, String>,
    pub timestamp: f64,
    pub value: f64,
}

impl PrometheusEndpoint {
    pub fn new(base_url: Url) -> Result<Self, MetricsError> {
        let client = PrometheusClient::try_from(base_url.as_str().to_owned()).map_err(|err| {
            MetricsError::new(format!("failed to create prometheus client: {err}"))
        })?;

        Ok(Self { base_url, client })
    }

    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.base_url.port_or_known_default()
    }

    pub fn instant_samples(
        &self,
        query: &str,
    ) -> Result<Vec<PrometheusInstantSample>, MetricsError> {
        let query = query.to_owned();
        let client = self.client.clone();

        let response = std::thread::spawn(move || -> Result<_, MetricsError> {
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|err| MetricsError::new(format!("failed to create runtime: {err}")))?;
            runtime
                .block_on(async { client.query(&query).get().await })
                .map_err(|err| MetricsError::new(format!("prometheus query failed: {err}")))
        })
        .join()
        .map_err(|_| MetricsError::new("prometheus query thread panicked"))??;

        let mut samples = Vec::new();
        match response.data() {
            PrometheusData::Vector(vectors) => {
                for vector in vectors {
                    samples.push(PrometheusInstantSample {
                        labels: vector.metric().clone(),
                        timestamp: vector.sample().timestamp(),
                        value: vector.sample().value(),
                    });
                }
            }
            PrometheusData::Matrix(ranges) => {
                for range in ranges {
                    let labels = range.metric().clone();
                    for sample in range.samples() {
                        samples.push(PrometheusInstantSample {
                            labels: labels.clone(),
                            timestamp: sample.timestamp(),
                            value: sample.value(),
                        });
                    }
                }
            }
            PrometheusData::Scalar(sample) => {
                samples.push(PrometheusInstantSample {
                    labels: HashMap::new(),
                    timestamp: sample.timestamp(),
                    value: sample.value(),
                });
            }
        }

        Ok(samples)
    }

    pub fn instant_values(&self, query: &str) -> Result<Vec<f64>, MetricsError> {
        self.instant_samples(query)
            .map(|samples| samples.into_iter().map(|sample| sample.value).collect())
    }

    pub fn counter_value(&self, query: &str) -> Result<f64, MetricsError> {
        self.instant_values(query)
            .map(|values| values.into_iter().sum())
    }
}
