use async_trait::async_trait;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{DynError, Expectation, RunContext},
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default)]
pub struct ConsensusLiveness;

const LAG_ALLOWANCE: u64 = 2;
const MIN_PROGRESS_BLOCKS: u64 = 5;

#[async_trait]
impl Expectation for ConsensusLiveness {
    fn name(&self) -> &'static str {
        "consensus_liveness"
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        let participants = Self::participants(ctx)?;
        let target_hint = Self::target_blocks(ctx);
        let check = Self::collect_results(participants).await;
        Self::report(target_hint, check)
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

    fn participants(ctx: &RunContext) -> Result<Vec<&ApiClient>, DynError> {
        let nodes: Vec<_> = ctx.node_clients().all_clients().collect();
        if nodes.is_empty() {
            Err(Box::new(ConsensusLivenessError::MissingParticipants))
        } else {
            Ok(nodes)
        }
    }

    async fn collect_results(nodes: Vec<&ApiClient>) -> LivenessCheck {
        let mut samples = Vec::with_capacity(nodes.len());
        let mut issues = Vec::new();

        for (index, client) in nodes.into_iter().enumerate() {
            let label = format!("node-{index}");
            match client.consensus_info().await {
                Ok(info) => samples.push(NodeSample {
                    label,
                    height: info.height,
                }),
                Err(err) => issues.push(ConsensusLivenessIssue::RequestFailed {
                    node: label,
                    source: err.into(),
                }),
            }
        }

        LivenessCheck { samples, issues }
    }

    fn report(target_hint: u64, mut check: LivenessCheck) -> Result<(), DynError> {
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
            if sample.height + LAG_ALLOWANCE < target {
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
