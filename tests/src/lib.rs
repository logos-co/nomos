pub mod nodes;
use once_cell::sync::Lazy;

// std
use std::env;
use std::net::TcpListener;
use std::ops::Mul;
use std::time::Duration;
use std::{fmt::Debug, sync::Mutex};

//crates
use nomos_libp2p::{Multiaddr, Swarm};
use rand::{thread_rng, Rng};

static NET_PORT: Lazy<Mutex<u16>> = Lazy::new(|| Mutex::new(thread_rng().gen_range(8000..10000)));
static IS_SLOW_TEST_ENV: Lazy<bool> =
    Lazy::new(|| env::var("SLOW_TEST_ENV").is_ok_and(|s| s == "true"));
pub static GLOBAL_PARAMS_PATH: Lazy<String> = Lazy::new(|| {
    let relative_path = "./kzgrs/kzgrs_test_params";
    let current_dir = env::current_dir().expect("Failed to get current directory");
    current_dir
        .join(relative_path)
        .canonicalize()
        .expect("Failed to resolve absolute path")
        .to_string_lossy()
        .to_string()
});

pub fn get_available_port() -> u16 {
    let mut port = NET_PORT.lock().unwrap();
    *port += 1;
    while TcpListener::bind(("127.0.0.1", *port)).is_err() {
        *port += 1;
    }
    *port
}

/// In slow test environments like Codecov, use 2x timeout.
pub fn adjust_timeout(d: Duration) -> Duration {
    if *IS_SLOW_TEST_ENV {
        d.mul(2)
    } else {
        d
    }
}

#[async_trait::async_trait]
pub trait Node: Sized {
    type Config: Send;
    type ConsensusInfo: Debug + Clone + PartialEq;
    async fn spawn(config: Self::Config) -> Self;
    async fn spawn_nodes(config: SpawnConfig) -> Vec<Self> {
        let mut nodes = Vec::new();
        for conf in Self::node_configs(config) {
            nodes.push(Self::spawn(conf).await);
        }
        nodes
    }
    fn node_configs(config: SpawnConfig) -> Vec<Self::Config>;
    fn create_node_configs(consensus: ConsensusConfig, da: DaConfig) -> Vec<Self::Config>;
    async fn consensus_info(&self) -> Self::ConsensusInfo;
    fn stop(&mut self);
}

#[derive(Clone)]
pub enum SpawnConfig {
    // Star topology: Every node is initially connected to a single node.
    Star {
        consensus: ConsensusConfig,
        n_executors: usize,
        da: DaConfig,
    },
    // Chain topology: Every node is chained to the node next to it.
    Chain {
        consensus: ConsensusConfig,
        n_executors: usize,
        da: DaConfig,
    },
}

impl SpawnConfig {
    pub fn n_executors(&self) -> usize {
        match self {
            SpawnConfig::Star { n_executors, .. } => *n_executors,
            SpawnConfig::Chain { n_executors, .. } => *n_executors,
        }
    }

    pub fn da_config(&self) -> DaConfig {
        match self {
            SpawnConfig::Star { da, .. } => da.clone(),
            SpawnConfig::Chain { da, .. } => da.clone(),
        }
    }
    // Returns a SpawnConfig::Chain with proper configurations for happy-path tests
    pub fn chain_happy(n_participants: usize, number_of_executors: usize, da: DaConfig) -> Self {
        Self::Chain {
            consensus: ConsensusConfig {
                n_participants,
                // by setting the active slot coeff close to 1, we also increase the probability of multiple blocks (forks)
                // being produced in the same slot (epoch). Setting the security parameter to some value > 1
                // ensures nodes have some time to sync before deciding on the longest chain.
                security_param: 10,
                // a block should be produced (on average) every slot
                active_slot_coeff: 0.9,
            },
            n_executors: number_of_executors,
            da,
        }
    }

    pub fn star_happy(n_participants: usize, number_of_executors: usize, da: DaConfig) -> Self {
        Self::Star {
            consensus: ConsensusConfig {
                n_participants,
                // by setting the slot coeff to 1, we also increase the probability of multiple blocks (forks)
                // being produced in the same slot (epoch). Setting the security parameter to some value > 1
                // ensures nodes have some time to sync before deciding on the longest chain.
                security_param: 10,
                // a block should be produced (on average) every slot
                active_slot_coeff: 0.9,
            },
            n_executors: number_of_executors,
            da,
        }
    }
}

fn node_address_from_port(port: u16) -> Multiaddr {
    Swarm::multiaddr(std::net::Ipv4Addr::new(127, 0, 0, 1), port)
}

#[derive(Clone)]
pub struct ConsensusConfig {
    pub n_participants: usize,
    pub security_param: u32,
    pub active_slot_coeff: f64,
}

#[derive(Clone)]
pub struct DaConfig {
    pub subnetwork_size: usize,
    pub dispersal_factor: usize,
    pub num_samples: u16,
    pub num_subnets: u16,
    pub old_blobs_check_interval: Duration,
    pub blobs_validity_duration: Duration,
    pub global_params_path: String,
}

impl Default for DaConfig {
    fn default() -> Self {
        Self {
            subnetwork_size: 2,
            dispersal_factor: 1,
            num_samples: 1,
            num_subnets: 2,
            old_blobs_check_interval: Duration::from_secs(5),
            blobs_validity_duration: Duration::from_secs(u64::MAX),
            global_params_path: GLOBAL_PARAMS_PATH.to_string(),
        }
    }
}
