use std::{fmt::Write as _, time::Duration};

use futures::FutureExt as _;

use super::{
    BoxFuture, CONSENSUS_PROCESSED_BLOCKS, Expectation, ExpectationError, MetricsError, RunContext,
};

/// Enforces that every validator advances to the minimum block height implied
/// by the scenario duration, slot timing, and active slot coefficient (or a
/// caller-provided override).
///
/// Polls each validator's HTTP consensus info to catch stalls even when
/// Prometheus is unavailable.
#[derive(Clone, Copy, Debug)]
pub struct ConsensusLiveness {
    minimum_override: Option<u64>,
    tolerance: f64,
}

pub struct PrometheusBlockProduction {
    minimum: u64,
}

impl PrometheusBlockProduction {
    #[must_use]
    pub const fn new(minimum: u64) -> Self {
        Self { minimum }
    }

    #[must_use]
    pub const fn minimum(&self) -> u64 {
        self.minimum
    }
}

impl Expectation for PrometheusBlockProduction {
    fn name(&self) -> &'static str {
        "prometheus_block_production"
    }

    fn evaluate<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), ExpectationError>> {
        async move {
            let total = ctx
                .metrics()
                .consensus_processed_blocks()
                .map_err(|err| into_expectation_error(&err))?;

            if total >= self.minimum() as f64 {
                tracing::info!(
                    query = CONSENSUS_PROCESSED_BLOCKS,
                    observed_total = total,
                    minimum = self.minimum(),
                    "block production expectation satisfied via prometheus"
                );
                Ok(())
            } else {
                Err(ExpectationError::new(format!(
                    "prometheus query `{}` sum {total} below block target {}",
                    CONSENSUS_PROCESSED_BLOCKS,
                    self.minimum()
                )))
            }
        }
        .boxed()
    }
}

fn into_expectation_error(err: &MetricsError) -> ExpectationError {
    ExpectationError::new(err.to_string())
}

impl ConsensusLiveness {
    #[must_use]
    pub const fn with_minimum(minimum_blocks: u64) -> Self {
        Self {
            minimum_override: Some(minimum_blocks),
            tolerance: 1.0,
        }
    }

    #[must_use]
    pub const fn with_tolerance(tolerance: f64) -> Self {
        Self {
            minimum_override: None,
            tolerance,
        }
    }
}

impl Default for ConsensusLiveness {
    fn default() -> Self {
        Self::with_tolerance(0.8)
    }
}

impl Expectation for ConsensusLiveness {
    fn name(&self) -> &'static str {
        "consensus_liveness"
    }

    fn evaluate<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), ExpectationError>> {
        async move {
            if ctx.validators().is_empty() {
                return Err(ExpectationError::new(
                    "consensus liveness requires at least one validator",
                ));
            }

            let target = consensus_target_blocks(ctx, self.minimum_override, self.tolerance);
            let mut issues = Vec::new();
            let mut heights = Vec::with_capacity(ctx.validators().len());

            for handle in ctx.validators() {
                let index = handle.descriptor().index;
                match handle.client().consensus_info().await {
                    Ok(info) => {
                        heights.push(info.height);
                        if info.height < target {
                            issues.push(format!(
                                "validator-{index} height {} below target {}",
                                info.height, target
                            ));
                        }
                    }
                    Err(err) => {
                        issues.push(format!("validator-{index} consensus_info failed: {err}"));
                    }
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
                let mut message = String::new();
                let _ = writeln!(
                    &mut message,
                    "consensus liveness violated (target={target}):"
                );
                for issue in issues {
                    let _ = writeln!(&mut message, "- {issue}");
                }
                Err(ExpectationError::new(message.trim_end()))
            }
        }
        .boxed()
    }
}

fn consensus_target_blocks(ctx: &RunContext, override_minimum: Option<u64>, tolerance: f64) -> u64 {
    if let Some(minimum) = override_minimum {
        return minimum;
    }

    if tolerance <= 0.0 {
        return 0;
    }

    let slot_duration = ctx
        .descriptors()
        .validators()
        .first()
        .map_or(Duration::from_secs(2), |node| {
            node.general.time_config.slot_duration
        });

    if slot_duration.is_zero() {
        return 0;
    }

    let active_slot_coeff = ctx
        .descriptors()
        .config()
        .consensus_params
        .active_slot_coeff;

    if active_slot_coeff <= 0.0 {
        return 0;
    }

    let run_duration = ctx.run_duration();
    if run_duration.is_zero() {
        return 0;
    }

    let slot_duration_secs = slot_duration.as_secs_f64();
    if slot_duration_secs == 0.0 {
        return 0;
    }

    let slot_count = run_duration.as_secs_f64() / slot_duration_secs;
    if slot_count < 1.0 {
        return 0;
    }

    let expected_blocks = slot_count * active_slot_coeff;
    let adjusted = (expected_blocks * tolerance).floor();
    adjusted.max(1.0) as u64
}
