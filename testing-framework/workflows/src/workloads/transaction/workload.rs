use std::{num::NonZeroU64, time::Duration};

use async_trait::async_trait;
use nomos_core::mantle::ops::channel::ChannelId;
use testing_framework_core::{
    scenario::{DynError, Expectation, RunContext, RunMetrics, Workload as ScenarioWorkload},
    topology::GeneratedTopology,
};
use thiserror::Error;

use super::expectation::TxInclusionExpectation;
use crate::util::tx;

const DEFAULT_TXS_PER_BLOCK: NonZeroU64 = NonZeroU64::new(5).unwrap();
#[derive(Clone)]
pub struct Workload {
    txs_per_block: NonZeroU64,
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "tx_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(self.inclusion_expectation()) as Box<dyn Expectation>]
    }

    fn init(
        &mut self,
        descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        let has_nodes = !descriptors.validators().is_empty() || !descriptors.executors().is_empty();
        if !has_nodes {
            return Err("tx workload requires at least one node in the topology".into());
        }

        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        self.submit_transactions(ctx).await
    }
}

impl Workload {
    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64) -> Self {
        Self { txs_per_block }
    }

    #[must_use]
    pub fn with_rate(txs_per_block: u64) -> Option<Self> {
        NonZeroU64::new(txs_per_block).map(Self::new)
    }

    #[must_use]
    pub const fn with_default_rate() -> Self {
        Self::new(DEFAULT_TXS_PER_BLOCK)
    }

    #[must_use]
    pub const fn inclusion_expectation(&self) -> TxInclusionExpectation {
        TxInclusionExpectation::new(self.txs_per_block)
    }

    async fn submit_transactions(&self, ctx: &RunContext) -> Result<(), DynError> {
        Submission::new(self, ctx)?.execute().await
    }
}

impl Default for Workload {
    fn default() -> Self {
        Self::with_default_rate()
    }
}

fn submission_plan(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
) -> Result<(usize, Duration), TxPlanError> {
    let (total_u64, total_usize) = planned_transaction_totals(txs_per_block, ctx)?;
    Ok((
        total_usize,
        transmission_interval(ctx.run_duration(), total_u64),
    ))
}

fn transmission_interval(run_duration: Duration, total_txs: u64) -> Duration {
    if total_txs == 0 {
        return Duration::ZERO;
    }

    let secs = run_duration.as_secs_f64();
    if !secs.is_finite() || secs <= 0.0 {
        return Duration::ZERO;
    }

    Duration::from_secs_f64(secs / total_txs as f64)
}

fn deterministic_channel_id(index: u64) -> ChannelId {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(b"tx_wrkld");
    bytes[24..].copy_from_slice(&index.to_be_bytes());
    ChannelId::from(bytes)
}

pub fn planned_channel_ids(total: usize) -> Vec<ChannelId> {
    (0..total as u64)
        .map(deterministic_channel_id)
        .collect::<Vec<_>>()
}

#[derive(Debug, Error)]
pub enum TxPlanError {
    #[error("tx workload total transactions exceed usize capacity")]
    CapacityOverflow,
}

pub fn planned_transaction_totals(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
) -> Result<(u64, usize), TxPlanError> {
    let window = ctx.run_duration();
    let window_secs = window.as_secs_f64();
    if window_secs <= 0.0 || !window_secs.is_finite() {
        return Ok((0, 0));
    }

    let metrics = ctx.run_metrics();
    let block_interval_secs = metrics
        .block_interval_hint()
        .map(|duration| duration.as_secs_f64())
        .filter(|secs| *secs > 0.0 && secs.is_finite())
        .unwrap_or(window_secs);

    if block_interval_secs <= 0.0 || !block_interval_secs.is_finite() {
        return Ok((0, 0));
    }

    let expected_blocks = window_secs / block_interval_secs;
    let total = expected_blocks * txs_per_block.get() as f64;
    let total_u64 = total.floor().clamp(0.0, u64::MAX as f64) as u64;
    let total_usize = usize::try_from(total_u64).map_err(|_| TxPlanError::CapacityOverflow)?;
    Ok((total_u64, total_usize))
}

struct Submission<'a> {
    plan: Vec<ChannelId>,
    ctx: &'a RunContext,
    interval: Duration,
}

impl<'a> Submission<'a> {
    fn new(workload: &Workload, ctx: &'a RunContext) -> Result<Self, DynError> {
        let (total, interval) = submission_plan(workload.txs_per_block, ctx)
            .map_err(|err| -> DynError { err.into() })?;
        let plan = planned_channel_ids(total);
        Ok(Self {
            plan,
            ctx,
            interval,
        })
    }

    async fn execute(self) -> Result<(), DynError> {
        for channel_id in self.plan {
            let client = self
                .ctx
                .random_node_client()
                .expect("node presence validated during initialization");
            let tx = tx::create_inscription_transaction_with_id(channel_id);
            client.submit_transaction(&tx).await?;
            if !self.interval.is_zero() {
                tokio::time::sleep(self.interval).await;
            }
        }
        Ok(())
    }
}
