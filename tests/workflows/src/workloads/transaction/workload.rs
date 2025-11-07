use std::{num::NonZeroU64, sync::Arc, time::Duration};

use async_trait::async_trait;
use nomos_core::mantle::ops::channel::ChannelId;
use rand::{RngCore as _, thread_rng};
use testing_framework_core::scenario::{
    DynError, Expectation, RunContext, Workload as ScenarioWorkload,
};

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

    fn expectations(&self) -> Vec<Arc<dyn Expectation>> {
        vec![Arc::new(self.inclusion_expectation()) as Arc<dyn Expectation>]
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
        let validator_clients = ctx.node_clients().validator_clients();
        if validator_clients.is_empty() {
            return Err("tx workload requires at least one validator".into());
        }

        let (total, interval) =
            submission_plan(self.txs_per_block, ctx).map_err(|err| -> DynError { err.into() })?;

        for client in validator_clients.iter().cycle().take(total) {
            let tx = tx::create_inscription_transaction_with_id(random_channel_id());
            client.submit_transaction(&tx).await?;

            if !interval.is_zero() {
                tokio::time::sleep(interval).await;
            }
        }

        Ok(())
    }
}

impl Default for Workload {
    fn default() -> Self {
        Self::with_default_rate()
    }
}

fn random_channel_id() -> ChannelId {
    let mut channel_id_bytes = [0u8; 32];
    thread_rng().fill_bytes(&mut channel_id_bytes);
    ChannelId::from(channel_id_bytes)
}

fn submission_plan(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
) -> Result<(usize, Duration), &'static str> {
    let blocks = planned_blocks(ctx);
    let total = blocks
        .checked_mul(txs_per_block.get())
        .ok_or("tx workload total transactions exceed u64 capacity")?;
    let total_usize = usize::try_from(total)
        .map_err(|_| "tx workload total transactions exceed usize capacity")?;

    Ok((
        total_usize,
        transmission_interval(ctx.run_duration(), total),
    ))
}

fn planned_blocks(ctx: &RunContext) -> u64 {
    ctx.expected_blocks().max(1)
}

fn transmission_interval(run_duration: Duration, total_txs: u64) -> Duration {
    if total_txs == 0 || run_duration.is_zero() {
        return Duration::ZERO;
    }

    let secs = run_duration.as_secs_f64();
    if !secs.is_finite() || secs <= 0.0 {
        return Duration::ZERO;
    }

    Duration::from_secs_f64(secs / total_txs as f64)
}
