use std::fmt;

use async_trait::async_trait;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{DynError, Expectation, RunContext},
    topology::NodeRole,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default)]
pub struct ConsensusLiveness;

const LAG_ALLOWANCE: u64 = 2;

#[async_trait]
impl Expectation for ConsensusLiveness {
    fn name(&self) -> &'static str {
        "consensus_liveness"
    }

    async fn evaluate(&self, ctx: &RunContext) -> Result<(), DynError> {
        let participants: Vec<_> = {
            let validator_pairs: Vec<_> = ctx
                .descriptors()
                .validators()
                .iter()
                .zip(ctx.node_clients().validator_clients().iter())
                .collect();
            if validator_pairs.is_empty() {
                ctx.descriptors()
                    .executors()
                    .iter()
                    .zip(ctx.node_clients().executor_clients().iter())
                    .collect()
            } else {
                validator_pairs
            }
        };

        if participants.is_empty() {
            return Err(Box::new(ConsensusLivenessError::MissingParticipants));
        }

        let target = Self::target_blocks(ctx);
        let mut issues = Vec::new();
        let mut heights = Vec::with_capacity(participants.len());

        for (descriptor, client) in participants {
            let label = NodeLabel::new(descriptor.role, descriptor.index);
            if let Some(issue) = participant_issue(label, client, target, &mut heights).await {
                issues.push(issue);
            }
        }

        if issues.is_empty() {
            tracing::info!(
                target,
                heights = ?heights,
                "consensus liveness expectation satisfied"
            );
            Ok(())
        } else {
            Err(Box::new(ConsensusLivenessError::Violations {
                target,
                details: issues.into(),
            }))
        }
    }
}

impl ConsensusLiveness {
    const fn target_blocks(ctx: &RunContext) -> u64 {
        consensus_target_blocks(ctx)
    }
}

const fn consensus_target_blocks(ctx: &RunContext) -> u64 {
    let expected_blocks = ctx.expected_blocks();
    if expected_blocks == 0 {
        return 0;
    }
    if expected_blocks <= LAG_ALLOWANCE {
        return expected_blocks;
    }
    expected_blocks - LAG_ALLOWANCE
}

async fn participant_issue(
    label: NodeLabel,
    client: &ApiClient,
    target: u64,
    heights: &mut Vec<u64>,
) -> Option<ConsensusLivenessIssue> {
    match record_participant(label, client, target, heights).await {
        Ok(maybe_issue) => maybe_issue,
        Err(issue) => Some(issue),
    }
}

async fn record_participant(
    label: NodeLabel,
    client: &ApiClient,
    target: u64,
    heights: &mut Vec<u64>,
) -> Result<Option<ConsensusLivenessIssue>, ConsensusLivenessIssue> {
    let height = consensus_height(label, client).await?;
    heights.push(height);
    Ok(shortfall_issue(label, height, target))
}

async fn consensus_height(
    label: NodeLabel,
    client: &ApiClient,
) -> Result<u64, ConsensusLivenessIssue> {
    client
        .consensus_info()
        .await
        .map(|info| info.height)
        .map_err(|err| ConsensusLivenessIssue::RequestFailed {
            node: label,
            source: err.into(),
        })
}

fn shortfall_issue(node: NodeLabel, height: u64, target: u64) -> Option<ConsensusLivenessIssue> {
    (height < target).then(|| ConsensusLivenessIssue::HeightBelowTarget {
        node,
        height,
        target,
    })
}

#[derive(Clone, Copy, Debug)]
struct NodeLabel {
    role: NodeRole,
    index: usize,
}

impl fmt::Display for NodeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.role {
            NodeRole::Validator => write!(f, "validator-{}", self.index),
            NodeRole::Executor => write!(f, "executor-{}", self.index),
        }
    }
}

impl NodeLabel {
    const fn new(role: NodeRole, index: usize) -> Self {
        Self { role, index }
    }
}

#[derive(Debug, Error)]
enum ConsensusLivenessIssue {
    #[error("{node} height {height} below target {target}")]
    HeightBelowTarget {
        node: NodeLabel,
        height: u64,
        target: u64,
    },
    #[error("{node} consensus_info failed: {source}")]
    RequestFailed {
        node: NodeLabel,
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
