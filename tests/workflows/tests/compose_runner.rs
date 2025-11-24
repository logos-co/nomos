use std::{env, time::Duration};

use serial_test::serial;
use testing_framework_core::scenario::{Deployer as _, Runner, ScenarioBuilder};
use testing_framework_runner_compose::{ComposeRunner, ComposeRunnerError};
use tests_workflows::{ChaosBuilderExt as _, ScenarioBuilderExt as _};

const RUN_DURATION: Duration = Duration::from_secs(120);
const MIXED_TXS_PER_BLOCK: u64 = 5;
const TOTAL_WALLETS: usize = 64;
const TRANSACTION_WALLETS: usize = 8;
const MAX_NODE_PAIR: usize = 6;

#[tokio::test]
#[serial]
async fn compose_runner_mixed_workloads() {
    for (validators, executors) in selected_node_pairs() {
        run_compose_case(validators, executors).await;
    }
}

fn selected_node_pairs() -> Vec<(usize, usize)> {
    if let Ok(raw) = env::var("COMPOSE_NODE_PAIRS") {
        return raw
            .split(',')
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| {
                let parts: Vec<_> = entry
                    .split(['x', 'X'])
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .collect();
                assert!(
                    parts.len() == 2,
                    "invalid COMPOSE_NODE_PAIRS entry '{entry}'; expected format '<v>x<e>'",
                );
                let validators = parts[0]
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("invalid validator count '{}'", parts[0]));
                let executors = parts[1]
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("invalid executor count '{}'", parts[1]));
                (validators, executors)
            })
            .collect();
    }

    (1..=MAX_NODE_PAIR).map(|n| (n, n)).collect()
}

async fn run_compose_case(validators: usize, executors: usize) {
    println!(
        "running compose chaos test with {validators} validator(s) and {executors} executor(s)"
    );

    let topology = ScenarioBuilder::with_node_counts(validators, executors)
        .enable_node_control()
        .chaos_random_restart()
        .min_delay(Duration::from_secs(45))
        .max_delay(Duration::from_secs(75))
        .apply()
        .topology()
        .validators(validators)
        .executors(executors)
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

    let mut plan = workloads
        .expect_consensus_liveness()
        .with_run_duration(RUN_DURATION)
        .build();

    let deployer = ComposeRunner::new().with_readiness(false);
    let runner: Runner = match deployer.deploy(&plan).await {
        Ok(runner) => runner,
        Err(ComposeRunnerError::DockerUnavailable) => {
            eprintln!("Skipping compose_runner_mixed_workloads: Docker is unavailable");
            return;
        }
        Err(err) => panic!("scenario deployment: {err}"),
    };
    let context = runner.context();
    assert!(
        context.telemetry().is_configured(),
        "compose runner should expose prometheus metrics"
    );

    let _handle = runner.run(&mut plan).await.expect("scenario executed");
}
