use std::time::Duration;

use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use thiserror::Error;
use tokio::time::sleep;

#[derive(Clone, Copy, Debug)]
pub struct ConsensusLiveness {
    lag_allowance: u64,
}

impl Default for ConsensusLiveness {
    fn default() -> Self {
        Self {
            lag_allowance: LAG_ALLOWANCE,
        }
    }
}

const LAG_ALLOWANCE: u64 = 2;
const MIN_PROGRESS_BLOCKS: u64 = 5;
const REQUEST_RETRIES: usize = 5;
const REQUEST_RETRY_DELAY: Duration = Duration::from_secs(2);

#[async_trait]
impl Expectation for ConsensusLiveness {
    fn name(&self) -> &'static str {
        "consensus_liveness"
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        Self::ensure_participants(ctx)?;
        let target_hint = Self::target_blocks(ctx);
        let check = Self::collect_results(ctx).await;
        (*self).report(target_hint, check)
    }
}

const fn consensus_target_blocks(ctx: &RunContext) -> u64 {
    ctx.expected_blocks()
}

#[derive(Debug, Error)]
enum ConsensusLivenessIssue {
    #[error("{node} height {height} below target {target}")]
    HeightBelowTarget {
        node: String,
        height: u64,
        target: u64,
    },
    #[error("{node} consensus_info failed: {source}")]
    RequestFailed {
        node: String,
        #[source]
        source: DynError,
    },
}

#[derive(Debug, Error)]
enum ConsensusLivenessError {
    #[error("consensus liveness requires at least one validator or executor")]
    MissingParticipants,
    #[error("consensus liveness violated (target={target}):\n{details}")]
    Violations {
        target: u64,
        #[source]
        details: ViolationIssues,
    },
}

#[derive(Debug, Error)]
#[error("{message}")]
struct ViolationIssues {
    issues: Vec<ConsensusLivenessIssue>,
    message: String,
}

impl ConsensusLiveness {
    const fn target_blocks(ctx: &RunContext) -> u64 {
        consensus_target_blocks(ctx)
    }

    fn ensure_participants(ctx: &RunContext) -> Result<(), DynError> {
        if ctx.node_clients().all_clients().count() == 0 {
            Err(Box::new(ConsensusLivenessError::MissingParticipants))
        } else {
            Ok(())
        }
    }

    async fn collect_results(ctx: &RunContext) -> LivenessCheck {
        let participant_count = ctx.node_clients().all_clients().count().max(1);
        let max_attempts = participant_count * REQUEST_RETRIES;
        let mut samples = Vec::with_capacity(participant_count);
        let mut issues = Vec::new();

        for attempt in 0..max_attempts {
            match Self::fetch_cluster_height(ctx).await {
                Ok(height) => {
                    samples.push(NodeSample {
                        label: format!("sample-{attempt}"),
                        height,
                    });
                    if samples.len() >= participant_count {
                        break;
                    }
                }
                Err(err) => issues.push(ConsensusLivenessIssue::RequestFailed {
                    node: format!("sample-{attempt}"),
                    source: err,
                }),
            }

            if samples.len() < participant_count {
                sleep(REQUEST_RETRY_DELAY).await;
            }
        }

        LivenessCheck { samples, issues }
    }

    async fn fetch_cluster_height(ctx: &RunContext) -> Result<u64, DynError> {
        ctx.cluster_client()
            .try_all_clients(|client| {
                Box::pin(async move {
                    client
                        .consensus_info()
                        .await
                        .map(|info| info.height)
                        .map_err(|err| -> DynError { err.into() })
                })
            })
            .await
    }

    #[must_use]
    pub const fn with_lag_allowance(mut self, lag_allowance: u64) -> Self {
        self.lag_allowance = lag_allowance;
        self
    }

    fn report(self, target_hint: u64, mut check: LivenessCheck) -> Result<(), DynError> {
        if check.samples.is_empty() {
            return Err(Box::new(ConsensusLivenessError::MissingParticipants));
        }

        let max_height = check
            .samples
            .iter()
            .map(|sample| sample.height)
            .max()
            .unwrap_or(0);

        let mut target = target_hint;
        if target == 0 || target > max_height {
            target = max_height;
        }

        if max_height < MIN_PROGRESS_BLOCKS {
            check
                .issues
                .push(ConsensusLivenessIssue::HeightBelowTarget {
                    node: "network".to_owned(),
                    height: max_height,
                    target: MIN_PROGRESS_BLOCKS,
                });
        }

        for sample in &check.samples {
            if sample.height + self.lag_allowance < target {
                check
                    .issues
                    .push(ConsensusLivenessIssue::HeightBelowTarget {
                        node: sample.label.clone(),
                        height: sample.height,
                        target,
                    });
            }
        }

        if check.issues.is_empty() {
            tracing::info!(
                target,
                heights = ?check.samples.iter().map(|s| s.height).collect::<Vec<_>>(),
                "consensus liveness expectation satisfied"
            );
            Ok(())
        } else {
            Err(Box::new(ConsensusLivenessError::Violations {
                target,
                details: check.issues.into(),
            }))
        }
    }
}

struct NodeSample {
    label: String,
    height: u64,
}

struct LivenessCheck {
    samples: Vec<NodeSample>,
    issues: Vec<ConsensusLivenessIssue>,
}

impl From<Vec<ConsensusLivenessIssue>> for ViolationIssues {
    fn from(issues: Vec<ConsensusLivenessIssue>) -> Self {
        let mut message = String::new();
        for issue in &issues {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str("- ");
            message.push_str(&issue.to_string());
        }
        Self { issues, message }
    }
}
