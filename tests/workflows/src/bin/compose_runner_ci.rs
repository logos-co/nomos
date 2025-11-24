use std::time::Duration;

use anyhow::{Context as _, Result};
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeRunner, ComposeRunnerError};
use tests_workflows::{
    ChaosBuilderExt as _, ScenarioBuilderExt as _, expectations::ConsensusLiveness,
};

const RUN_DURATION: Duration = Duration::from_secs(120);
const VALIDATORS: usize = 2;
const EXECUTORS: usize = 2;
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;

#[tokio::main]
async fn main() -> Result<()> {
    let topology = ScenarioBuilder::with_node_counts(VALIDATORS, EXECUTORS)
        .enable_node_control()
        .chaos_random_restart()
        .min_delay(Duration::from_secs(45))
        .max_delay(Duration::from_secs(75))
        .target_cooldown(Duration::from_secs(120))
        .apply()
        .topology()
        .validators(VALIDATORS)
        .executors(EXECUTORS)
        .network_star()
        .apply();

    let workloads = topology
        .wallets(TOTAL_WALLETS)
        .transactions()
        .rate(MIXED_TXS_PER_BLOCK)
        .users(TRANSACTION_WALLETS)
        .apply()
        .da()
        .rate(1)
        .blob_rate(1)
        .apply();

    let lag_allowance = 2 + (VALIDATORS + EXECUTORS) as u64;
    let mut plan = workloads
        .with_expectation(ConsensusLiveness::default().with_lag_allowance(lag_allowance))
        .with_run_duration(RUN_DURATION)
        .build();

    let deployer = ComposeRunner::new().with_readiness(false);
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            anyhow::bail!("Docker is required for compose runner CI binary");
        }
        Err(err) => return Err(err.into()),
    };

    if !runner.context().telemetry().is_configured() {
        anyhow::bail!("compose runner should expose prometheus metrics");
    }

    runner
        .run(&mut plan)
        .await
        .context("compose scenario execution failed")?;

    Ok(())
}
