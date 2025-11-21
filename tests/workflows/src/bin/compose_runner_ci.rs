use std::time::Duration;

use anyhow::{Context as _, Result};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::ComposeRunner;
use tests_workflows::ScenarioBuilderExt as _;

const RUN_DURATION: Duration = Duration::from_secs(60);
const VALIDATORS: usize = 1;
const EXECUTORS: usize = 1;
const MIXED_TXS_PER_BLOCK: u64 = 2;

#[tokio::main]
async fn main() -> Result<()> {
    let topology = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .topology()
        .validators(VALIDATORS)
        .executors(EXECUTORS)
        .network_star()
        .apply();

    let workloads = topology
        .transactions()
        .rate(MIXED_TXS_PER_BLOCK)
        .apply()
        .da()
        .rate(1)
        .blob_rate(1)
        .apply();

    let mut plan = workloads.with_run_duration(RUN_DURATION).build();

    let deployer = ComposeRunner::new().with_readiness(false);
    let runner: Runner = deployer
        .deploy(&plan)
        .await
        .context("compose runner deployment failed")?;

    if !runner.context().telemetry().is_configured() {
        anyhow::bail!("compose runner should expose prometheus metrics");
    }

    let runner = runner;
    runner
        .run(&mut plan)
        .await
        .context("compose scenario execution failed")?;

    Ok(())
}
