//! Runs a small local cluster for a fixed time with the given consensus and
//! Blend parameters, so the block-delay and reorg instrumentation can be
//! collected from the node log files afterwards.
//!
//! The analysis scripts that consume the logs live in `tools/block-times-lab`.
//!
//! ```text
//! block-times-lab --nodes 2 --minutes 20 --slot-secs 1 --f 1/4 \
//!     --blend-layers 1 --release-delay-rounds 3 --security-param 10 --out DIR
//! ```

use std::{
    collections::HashMap,
    env, fs,
    num::{NonZero, NonZeroU64},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lb_node::config::{
    RunConfig,
    tracing::serde::{Level, filter, metrics},
};
use lb_testing_framework::{DeploymentBuilder, LbcEnv, TopologyConfig};
use lb_utils::math::NonNegativeRatio;
use logos_blockchain_tests::common::manual_cluster::{
    ManualNodeLayout, api_url, read_manual_node_logs, start_local_manual_cluster_with_layout,
};
use testing_framework_core::scenario::{DynError, StartedNode};
use tokio::time::sleep;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
struct Params {
    nodes: usize,
    minutes: u64,
    slot_secs: u64,
    f_num: u32,
    f_den: u32,
    blend_layers: u64,
    release_delay_rounds: u64,
    security_param: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            nodes: 2,
            minutes: 20,
            slot_secs: 1,
            f_num: 1,
            f_den: 4,
            blend_layers: 1,
            release_delay_rounds: 3,
            security_param: 10,
        }
    }
}

fn parse_args() -> Result<(Params, PathBuf), String> {
    let mut params = Params::default();
    let mut out = PathBuf::from("block-times-lab-runs");
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let number = |what: &str| value.parse::<u64>().map_err(|e| format!("{what}: {e}"));
        match flag.as_str() {
            "--nodes" => params.nodes = usize::try_from(number("--nodes")?).unwrap_or(2),
            "--minutes" => params.minutes = number("--minutes")?,
            "--slot-secs" => params.slot_secs = number("--slot-secs")?,
            "--f" => {
                let (num, den) = value
                    .split_once('/')
                    .ok_or("--f expects NUM/DEN, e.g. 1/4")?;
                params.f_num = num.parse().map_err(|e| format!("--f numerator: {e}"))?;
                params.f_den = den.parse().map_err(|e| format!("--f denominator: {e}"))?;
            }
            "--blend-layers" => params.blend_layers = number("--blend-layers")?,
            "--release-delay-rounds" => {
                params.release_delay_rounds = number("--release-delay-rounds")?;
            }
            "--security-param" => {
                params.security_param =
                    u32::try_from(number("--security-param")?).map_err(|e| e.to_string())?;
            }
            "--out" => out = PathBuf::from(value),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok((params, out))
}

/// Applies the lab parameters to every node's run config: the chain
/// parameters go into the genesis era, and logging is pinned to INFO for
/// the node's own targets so the instrumentation events land in the log file.
fn patch(mut config: RunConfig, params: Params) -> RunConfig {
    let era = config.deployment.genesis_era_parameters_mut();
    era.time.slot_duration = Duration::from_secs(params.slot_secs);
    era.cryptarchia.security_param = NonZero::new(params.security_param).expect("nonzero");
    era.cryptarchia.slot_activation_coeff =
        NonNegativeRatio::new(params.f_num, NonZero::new(params.f_den).expect("nonzero"));
    era.blend.common.num_blend_layers = NonZeroU64::new(params.blend_layers).expect("nonzero");
    era.blend
        .core
        .scheduler
        .delayer
        .maximum_release_delay_in_rounds =
        NonZeroU64::new(params.release_delay_rounds).expect("nonzero");

    config.user.tracing.metrics = metrics::Layer::none().with_open_metrics(true);
    config.user.tracing.level = Level::INFO;
    config.user.tracing.filter = filter::Layer::Env(filter::EnvConfig {
        filters: HashMap::from([
            ("logos_blockchain".to_owned(), Level::INFO),
            ("libp2p".to_owned(), Level::WARN),
        ]),
    });
    config
}

type Node = StartedNode<LbcEnv>;

/// Prints every node's tip every [`PROGRESS_INTERVAL`] until `run_for` has
/// elapsed.
async fn watch_progress(nodes: &[Node], run_for: Duration) {
    let total_secs = run_for.as_secs();
    let deadline = Instant::now() + run_for;
    while Instant::now() < deadline {
        sleep(PROGRESS_INTERVAL.min(deadline.saturating_duration_since(Instant::now()))).await;
        let elapsed = total_secs - deadline.saturating_duration_since(Instant::now()).as_secs();
        for node in nodes {
            match node.client.consensus_info().await {
                Ok(info) => println!(
                    "[{elapsed:>5}s] {}: phase={:?} height={} slot={} lib_slot={}",
                    node.name,
                    info.phase,
                    info.cryptarchia_info.height,
                    u64::from(info.cryptarchia_info.slot),
                    u64::from(info.cryptarchia_info.lib_slot),
                ),
                Err(error) => println!("[{elapsed:>5}s] {}: info failed: {error}", node.name),
            }
        }
    }
}

/// Proves the metrics exist: scrapes each node's `OpenMetrics` document into
/// `metrics-<node>.txt`.
async fn scrape_metrics(nodes: &[Node], scenario_dir: &Path) {
    for node in nodes {
        let url = api_url(&node.client, "metrics");
        let target = scenario_dir.join(format!("metrics-{}.txt", node.name));
        match reqwest::get(url.clone()).await {
            Ok(response) if response.status().is_success() => match response.text().await {
                Ok(document) => {
                    let lines = document.lines().filter(|l| !l.starts_with('#')).count();
                    if let Err(error) = fs::write(&target, document) {
                        eprintln!("{}: could not write metrics: {error}", node.name);
                    } else {
                        println!(
                            "{}: {lines} metric samples -> {}",
                            node.name,
                            target.display()
                        );
                    }
                }
                Err(error) => eprintln!("{}: metrics body unreadable: {error}", node.name),
            },
            Ok(response) => eprintln!("{}: metrics returned {}", node.name, response.status()),
            Err(error) => eprintln!("{}: metrics request failed: {error}", node.name),
        }
    }
}

/// The framework removes the node runtime directories on shutdown, so the
/// log files are copied into the scenario directory first.
fn snapshot_logs(nodes: &[Node], scenario_dir: &Path) {
    for node in nodes {
        let logs = read_manual_node_logs(scenario_dir, &node.name);
        let node_dir = scenario_dir.join(&node.name);
        let target = node_dir.join(format!("__logs-{}", node.name));
        match fs::create_dir_all(&node_dir).and_then(|()| fs::write(&target, &logs)) {
            Ok(()) => println!(
                "{}: {} log bytes -> {}",
                node.name,
                logs.len(),
                target.display()
            ),
            Err(error) => eprintln!("{}: could not snapshot logs: {error}", node.name),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let (params, out) = match parse_args() {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    fs::create_dir_all(&out).expect("output directory should be creatable");
    let out = out.canonicalize().expect("output directory should resolve");
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let test_name = format!("lab-{started_unix}");

    println!("starting {} nodes: {params:?}", params.nodes);
    let mut topology =
        TopologyConfig::with_node_numbers(params.nodes).with_test_context(Some(test_name.clone()));
    topology.slot_duration = Some(Duration::from_secs(params.slot_secs));
    let builder = DeploymentBuilder::new(topology)
        .with_security_param(NonZero::new(params.security_param).expect("nonzero"))
        .with_slot_activation_coeff(params.f_num, NonZero::new(params.f_den).expect("nonzero"));

    let (base, nodes) = start_local_manual_cluster_with_layout(
        &test_name,
        "block-times-lab",
        builder,
        params.nodes,
        ManualNodeLayout::SelectNodeSeed(0),
        move |config| Ok::<_, DynError>(patch(config, params)),
        Some(out),
    )
    .await;

    let scenario_dir = base.scenario_base_dir().to_path_buf();
    let time_info = match nodes[0].client.time_info().await {
        Ok(info) => serde_json::json!({
            "slot_duration_ms": info.slot_duration_ms,
            "genesis_time_unix_ms": info.genesis_time_unix_ms,
            "slots_per_epoch": info.slots_per_epoch,
        }),
        Err(error) => {
            eprintln!("time info unavailable: {error}");
            serde_json::Value::Null
        }
    };
    let manifest = serde_json::json!({
        "time": time_info,
        "nodes": params.nodes,
        "minutes": params.minutes,
        "slot_secs": params.slot_secs,
        "f": format!("{}/{}", params.f_num, params.f_den),
        "f_value": f64::from(params.f_num) / f64::from(params.f_den),
        "blend_layers": params.blend_layers,
        "release_delay_rounds": params.release_delay_rounds,
        "security_param": params.security_param,
        "started_unix": started_unix,
    });
    fs::write(
        scenario_dir.join("lab.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest serializes"),
    )
    .expect("manifest should be writable");
    println!("scenario dir: {}", scenario_dir.display());

    watch_progress(&nodes, Duration::from_secs(params.minutes * 60)).await;

    scrape_metrics(&nodes, &scenario_dir).await;
    snapshot_logs(&nodes, &scenario_dir);

    println!("stopping nodes");
    base.cluster().stop_all();
    println!("done; logs are under {}", scenario_dir.display());
    ExitCode::SUCCESS
}
