pub mod configs {
    pub use integration_configs::topology::configs::*;
}

use std::{
    collections::{HashMap, HashSet},
    iter,
    time::Duration,
};

use configs::{
    GeneralConfig,
    consensus::{ProviderInfo, create_genesis_tx_with_declarations},
    da::{DaParams, create_da_configs},
    network::{NetworkParams, create_network_configs},
    tracing::create_tracing_configs,
};
use futures::future::join_all;
use groth16::fr_to_bytes;
use key_management_system::{
    backend::preload::PreloadKMSBackendSettings,
    keys::{Ed25519Key, Key, ZkKey},
};
use nomos_core::{
    mantle::GenesisTx as _,
    sdp::{Locator, ServiceType, SessionNumber},
};
use nomos_da_network_core::swarm::DAConnectionPolicySettings;
use nomos_da_network_service::MembershipResponse;
use nomos_http_api_common::paths;
use nomos_network::backends::libp2p::Libp2pInfo;
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use reqwest::{Client, Url};
use thiserror::Error;
use tokio::time::{sleep, timeout};
use tracing::warn;

use crate::{
    adjust_timeout,
    nodes::{
        executor::{Executor, create_executor_config},
        validator::{Validator, create_validator_config},
    },
    topology::configs::{
        api::create_api_configs,
        blend::{GeneralBlendConfig, create_blend_configs},
        bootstrap::{SHORT_PROLONGED_BOOTSTRAP_PERIOD, create_bootstrap_configs},
        consensus::{ConsensusParams, create_consensus_configs},
        da::GeneralDaConfig,
        time::default_time_config,
    },
};

#[derive(Clone)]
pub struct TopologyConfig {
    pub n_validators: usize,
    pub n_executors: usize,
    pub consensus_params: ConsensusParams,
    pub da_params: DaParams,
    pub network_params: NetworkParams,
}

impl TopologyConfig {
    #[must_use]
    pub fn two_validators() -> Self {
        Self {
            n_validators: 2,
            n_executors: 0,
            consensus_params: ConsensusParams::default_for_participants(2),
            da_params: DaParams::default(),
            network_params: NetworkParams::default(),
        }
    }

    #[must_use]
    pub fn validator_and_executor() -> Self {
        Self {
            n_validators: 1,
            n_executors: 1,
            consensus_params: ConsensusParams::default_for_participants(2),
            da_params: DaParams {
                dispersal_factor: 2,
                subnetwork_size: 2,
                num_subnets: 2,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: 1,
                    min_replication_peers: 1,
                    max_dispersal_failures: 0,
                    max_sampling_failures: 0,
                    max_replication_failures: 0,
                    malicious_threshold: 0,
                },
                balancer_interval: Duration::from_secs(1),
                ..Default::default()
            },
            network_params: NetworkParams::default(),
        }
    }

    #[must_use]
    pub fn with_node_numbers(validators: usize, executors: usize) -> Self {
        let participants = validators + executors;
        assert!(participants > 0, "topology must include at least one node");

        let mut da_params = DaParams::default();
        let da_nodes = participants;
        if da_nodes <= 1 {
            da_params.subnetwork_size = 1;
            da_params.num_subnets = 1;
            da_params.dispersal_factor = 1;
            da_params.policy_settings.min_dispersal_peers = 0;
            da_params.policy_settings.min_replication_peers = 0;
        } else {
            let dispersal = da_nodes.min(da_params.dispersal_factor.max(2));
            da_params.dispersal_factor = dispersal;
            da_params.subnetwork_size = da_params.subnetwork_size.max(dispersal);
            da_params.num_subnets = da_params.subnetwork_size as u16;
            let min_peers = dispersal.saturating_sub(1).max(1);
            da_params.policy_settings.min_dispersal_peers = min_peers;
            da_params.policy_settings.min_replication_peers = min_peers;
            da_params.balancer_interval = Duration::from_secs(1);
        }

        Self {
            n_validators: validators,
            n_executors: executors,
            consensus_params: ConsensusParams::default_for_participants(participants),
            da_params,
            network_params: NetworkParams::default(),
        }
    }

    #[must_use]
    pub fn validators_and_executor(
        num_validators: usize,
        num_subnets: usize,
        dispersal_factor: usize,
    ) -> Self {
        Self {
            n_validators: num_validators,
            n_executors: 1,
            consensus_params: ConsensusParams::default_for_participants(num_validators + 1),
            da_params: DaParams {
                dispersal_factor,
                subnetwork_size: num_subnets,
                num_subnets: num_subnets as u16,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: num_subnets,
                    min_replication_peers: dispersal_factor - 1,
                    max_dispersal_failures: 0,
                    max_sampling_failures: 0,
                    max_replication_failures: 0,
                    malicious_threshold: 0,
                },
                balancer_interval: Duration::from_secs(5),
                ..Default::default()
            },
            network_params: NetworkParams::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
    Validator,
    Executor,
}

#[derive(Clone)]
pub struct GeneratedNodeConfig {
    pub role: NodeRole,
    pub index: usize,
    pub id: [u8; 32],
    pub general: GeneralConfig,
    pub da_port: u16,
    pub blend_port: u16,
}

impl GeneratedNodeConfig {
    #[must_use]
    pub const fn role(&self) -> NodeRole {
        self.role
    }

    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn network_port(&self) -> u16 {
        self.general.network_config.swarm_config.port
    }

    #[must_use]
    pub const fn api_port(&self) -> u16 {
        self.general.api_config.address.port()
    }

    #[must_use]
    pub const fn testing_http_port(&self) -> u16 {
        self.general.api_config.testing_http_address.port()
    }
}

#[derive(Clone)]
pub struct GeneratedTopology {
    config: TopologyConfig,
    validators: Vec<GeneratedNodeConfig>,
    executors: Vec<GeneratedNodeConfig>,
}

impl GeneratedTopology {
    #[must_use]
    pub const fn config(&self) -> &TopologyConfig {
        &self.config
    }

    #[must_use]
    pub fn validators(&self) -> &[GeneratedNodeConfig] {
        &self.validators
    }

    #[must_use]
    pub fn executors(&self) -> &[GeneratedNodeConfig] {
        &self.executors
    }

    pub fn nodes(&self) -> impl Iterator<Item = &GeneratedNodeConfig> {
        self.validators.iter().chain(self.executors.iter())
    }

    pub async fn spawn_local(&self) -> Topology {
        let configs = self
            .nodes()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let (validators, executors) = Topology::spawn_validators_executors(
            configs,
            self.config.n_validators,
            self.config.n_executors,
        )
        .await;

        Topology {
            validators,
            executors,
        }
    }

    pub async fn wait_remote_readiness(
        &self,
        validator_endpoints: &[Url],
        executor_endpoints: &[Url],
        validator_membership_endpoints: Option<&[Url]>,
        executor_membership_endpoints: Option<&[Url]>,
    ) -> Result<(), ReadinessError> {
        let total_nodes = self.validators.len() + self.executors.len();
        if total_nodes == 0 {
            return Ok(());
        }

        assert_eq!(
            self.validators.len(),
            validator_endpoints.len(),
            "validator endpoints must match topology"
        );
        assert_eq!(
            self.executors.len(),
            executor_endpoints.len(),
            "executor endpoints must match topology"
        );

        let mut endpoints = Vec::with_capacity(total_nodes);
        endpoints.extend_from_slice(validator_endpoints);
        endpoints.extend_from_slice(executor_endpoints);

        let labels = self.labels();
        let client = Client::new();
        let make_testing_base_url = |port: u16| -> Url {
            Url::parse(&format!("http://127.0.0.1:{port}/"))
                .expect("failed to construct local testing base url")
        };

        if endpoints.len() > 1 {
            let listen_ports = self.listen_ports();
            let initial_peer_ports = self.initial_peer_ports();
            let expected_peer_counts =
                find_expected_peer_counts(&listen_ports, &initial_peer_ports);
            let network_check = HttpNetworkReadiness {
                client: &client,
                endpoints: &endpoints,
                expected_peer_counts: &expected_peer_counts,
                labels: &labels,
            };

            network_check.wait().await?;
        }

        let mut membership_endpoints = Vec::with_capacity(total_nodes);
        if let Some(urls) = validator_membership_endpoints {
            assert_eq!(
                self.validators.len(),
                urls.len(),
                "validator membership endpoints must match topology"
            );
            membership_endpoints.extend_from_slice(urls);
        } else {
            membership_endpoints.extend(
                self.validators
                    .iter()
                    .map(|node| make_testing_base_url(node.testing_http_port())),
            );
        }

        if let Some(urls) = executor_membership_endpoints {
            assert_eq!(
                self.executors.len(),
                urls.len(),
                "executor membership endpoints must match topology"
            );
            membership_endpoints.extend_from_slice(urls);
        } else {
            membership_endpoints.extend(
                self.executors
                    .iter()
                    .map(|node| make_testing_base_url(node.testing_http_port())),
            );
        }

        let membership_check = HttpMembershipReadiness {
            client: &client,
            endpoints: &membership_endpoints,
            session: SessionNumber::from(0u64),
            labels: &labels,
            expect_non_empty: true,
        };

        membership_check.wait().await
    }

    fn listen_ports(&self) -> Vec<u16> {
        self.validators
            .iter()
            .map(|node| node.general.network_config.swarm_config.port)
            .chain(
                self.executors
                    .iter()
                    .map(|node| node.general.network_config.swarm_config.port),
            )
            .collect()
    }

    fn initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.validators
            .iter()
            .map(|node| {
                node.general
                    .network_config
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            })
            .chain(self.executors.iter().map(|node| {
                node.general
                    .network_config
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            }))
            .collect()
    }

    fn labels(&self) -> Vec<String> {
        self.validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "validator#{idx}@{}",
                    node.general.network_config.swarm_config.port
                )
            })
            .chain(self.executors.iter().enumerate().map(|(idx, node)| {
                format!(
                    "executor#{idx}@{}",
                    node.general.network_config.swarm_config.port
                )
            }))
            .collect()
    }
}

#[derive(Clone)]
pub struct TopologyBuilder {
    config: TopologyConfig,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
}

impl TopologyBuilder {
    #[must_use]
    pub const fn new(config: TopologyConfig) -> Self {
        Self {
            config,
            ids: None,
            da_ports: None,
            blend_ports: None,
        }
    }

    #[must_use]
    pub fn with_ids(mut self, ids: Vec<[u8; 32]>) -> Self {
        self.ids = Some(ids);
        self
    }

    #[must_use]
    pub fn with_da_ports(mut self, ports: Vec<u16>) -> Self {
        self.da_ports = Some(ports);
        self
    }

    #[must_use]
    pub fn with_blend_ports(mut self, ports: Vec<u16>) -> Self {
        self.blend_ports = Some(ports);
        self
    }

    #[must_use]
    pub fn build(self) -> GeneratedTopology {
        let Self {
            config,
            ids,
            da_ports,
            blend_ports,
        } = self;

        let n_participants = config.n_validators + config.n_executors;
        assert!(n_participants > 0, "topology must have at least one node");

        let ids = resolve_ids(ids, n_participants);
        let da_ports = resolve_ports(da_ports, n_participants, "DA");
        let blend_ports = resolve_ports(blend_ports, n_participants, "Blend");

        let mut consensus_configs = create_consensus_configs(&ids, &config.consensus_params);
        let bootstrapping_config = create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
        let da_configs = create_da_configs(&ids, &config.da_params, &da_ports);
        let network_configs = create_network_configs(&ids, &config.network_params);
        let blend_configs = create_blend_configs(&ids, &blend_ports);
        let api_configs = create_api_configs(&ids);
        let tracing_configs = create_tracing_configs(&ids);
        let time_config = default_time_config();

        let mut providers: Vec<_> = da_configs
            .iter()
            .enumerate()
            .map(|(i, da_conf)| ProviderInfo {
                service_type: ServiceType::DataAvailability,
                provider_sk: da_conf.signer.clone(),
                zk_sk: da_conf.secret_zk_key.clone(),
                locator: Locator(da_conf.listening_address.clone()),
                note: consensus_configs[0].da_notes[i].clone(),
            })
            .collect();
        providers.extend(
            blend_configs
                .iter()
                .enumerate()
                .map(|(i, blend_conf)| ProviderInfo {
                    service_type: ServiceType::BlendNetwork,
                    provider_sk: blend_conf.signer.clone(),
                    zk_sk: blend_conf.secret_zk_key.clone(),
                    locator: Locator(blend_conf.backend_core.listening_address.clone()),
                    note: consensus_configs[0].blend_notes[i].clone(),
                }),
        );

        let ledger_tx = consensus_configs[0]
            .genesis_tx
            .mantle_tx()
            .ledger_tx
            .clone();
        let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
        for c in &mut consensus_configs {
            c.genesis_tx = genesis_tx.clone();
        }

        let kms_configs = create_kms_configs(&blend_configs, &da_configs);

        let mut validators = Vec::with_capacity(config.n_validators);
        let mut executors = Vec::with_capacity(config.n_executors);

        for i in 0..n_participants {
            let general = GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrapping_config[i].clone(),
                da_config: da_configs[i].clone(),
                network_config: network_configs[i].clone(),
                blend_config: blend_configs[i].clone(),
                api_config: api_configs[i].clone(),
                tracing_config: tracing_configs[i].clone(),
                time_config: time_config.clone(),
                kms_config: kms_configs[i].clone(),
            };

            let role = if i < config.n_validators {
                NodeRole::Validator
            } else {
                NodeRole::Executor
            };
            let index = match role {
                NodeRole::Validator => i,
                NodeRole::Executor => i - config.n_validators,
            };

            let descriptor = GeneratedNodeConfig {
                role,
                index,
                id: ids[i],
                general,
                da_port: da_ports[i],
                blend_port: blend_ports[i],
            };

            match role {
                NodeRole::Validator => validators.push(descriptor),
                NodeRole::Executor => executors.push(descriptor),
            }
        }

        GeneratedTopology {
            config,
            validators,
            executors,
        }
    }
}

pub struct Topology {
    validators: Vec<Validator>,
    executors: Vec<Executor>,
}

impl Topology {
    pub async fn spawn(config: TopologyConfig) -> Self {
        let n_participants = config.n_validators + config.n_executors;

        // we use the same random bytes for:
        // * da id
        // * coin sk
        // * coin nonce
        // * libp2p node key
        let mut ids = vec![[0; 32]; n_participants];
        let mut da_ports = vec![];
        let mut blend_ports = vec![];
        for id in &mut ids {
            thread_rng().fill(id);
            da_ports.push(get_available_udp_port().unwrap());
            blend_ports.push(get_available_udp_port().unwrap());
        }

        let mut consensus_configs = create_consensus_configs(&ids, &config.consensus_params);
        let bootstrapping_config = create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
        let da_configs = create_da_configs(&ids, &config.da_params, &da_ports);
        let network_configs = create_network_configs(&ids, &config.network_params);
        let blend_configs = create_blend_configs(&ids, &blend_ports);
        let api_configs = create_api_configs(&ids);
        let tracing_configs = create_tracing_configs(&ids);
        let time_config = default_time_config();

        // Setup genesis TX with Blend and DA service declarationse
        let mut providers: Vec<_> = da_configs
            .iter()
            .enumerate()
            .map(|(i, da_conf)| ProviderInfo {
                service_type: ServiceType::DataAvailability,
                provider_sk: da_conf.signer.clone(),
                zk_sk: da_conf.secret_zk_key.clone(),
                locator: Locator(da_conf.listening_address.clone()),
                note: consensus_configs[0].da_notes[i].clone(),
            })
            .collect();
        providers.extend(
            blend_configs
                .iter()
                .enumerate()
                .map(|(i, blend_conf)| ProviderInfo {
                    service_type: ServiceType::BlendNetwork,
                    provider_sk: blend_conf.signer.clone(),
                    zk_sk: blend_conf.secret_zk_key.clone(),
                    locator: Locator(blend_conf.backend_core.listening_address.clone()),
                    note: consensus_configs[0].blend_notes[i].clone(),
                }),
        );

        // Update genesis TX to contain Blend and DA providers.
        let ledger_tx = consensus_configs[0]
            .genesis_tx
            .mantle_tx()
            .ledger_tx
            .clone();
        let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
        for c in &mut consensus_configs {
            c.genesis_tx = genesis_tx.clone();
        }

        // Set Blend and DA keys in KMS of each node config.
        let kms_configs = create_kms_configs(&blend_configs, &da_configs);

        let mut node_configs = vec![];

        for i in 0..n_participants {
            node_configs.push(GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrapping_config[i].clone(),
                da_config: da_configs[i].clone(),
                network_config: network_configs[i].clone(),
                blend_config: blend_configs[i].clone(),
                api_config: api_configs[i].clone(),
                tracing_config: tracing_configs[i].clone(),
                time_config: time_config.clone(),
                kms_config: kms_configs[i].clone(),
            });
        }

        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, config.n_validators, config.n_executors)
                .await;

        Self {
            validators,
            executors,
        }
    }

    pub async fn spawn_with_empty_membership(
        config: TopologyConfig,
        ids: &[[u8; 32]],
        da_ports: &[u16],
        blend_ports: &[u16],
    ) -> Self {
        let n_participants = config.n_validators + config.n_executors;

        let consensus_configs = create_consensus_configs(ids, &config.consensus_params);
        let bootstrapping_config = create_bootstrap_configs(ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
        let da_configs = create_da_configs(ids, &config.da_params, da_ports);
        let network_configs = create_network_configs(ids, &config.network_params);
        let blend_configs = create_blend_configs(ids, blend_ports);
        let api_configs = create_api_configs(ids);
        // Create membership configs without DA nodes.
        let tracing_configs = create_tracing_configs(ids);
        let time_config = default_time_config();

        let kms_config = PreloadKMSBackendSettings {
            keys: HashMap::new(),
        };

        let mut node_configs = vec![];

        for i in 0..n_participants {
            node_configs.push(GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrapping_config[i].clone(),
                da_config: da_configs[i].clone(),
                network_config: network_configs[i].clone(),
                blend_config: blend_configs[i].clone(),
                api_config: api_configs[i].clone(),
                tracing_config: tracing_configs[i].clone(),
                time_config: time_config.clone(),
                kms_config: kms_config.clone(),
            });
        }
        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, config.n_validators, config.n_executors)
                .await;

        Self {
            validators,
            executors,
        }
    }

    async fn spawn_validators_executors(
        config: Vec<GeneralConfig>,
        n_validators: usize,
        n_executors: usize,
    ) -> (Vec<Validator>, Vec<Executor>) {
        let mut validators = Vec::new();
        for i in 0..n_validators {
            let config = create_validator_config(config[i].clone());
            validators.push(Validator::spawn(config).await.unwrap());
        }

        let mut executors = Vec::new();
        for i in n_validators..(n_validators + n_executors) {
            let config = create_executor_config(config[i].clone());
            executors.push(Executor::spawn(config).await);
        }

        (validators, executors)
    }

    #[must_use]
    pub fn validators(&self) -> &[Validator] {
        &self.validators
    }

    #[must_use]
    pub fn executors(&self) -> &[Executor] {
        &self.executors
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let listen_ports = self.node_listen_ports();
        if listen_ports.len() <= 1 {
            return Ok(());
        }

        let initial_peer_ports = self.node_initial_peer_ports();
        let expected_peer_counts = find_expected_peer_counts(&listen_ports, &initial_peer_ports);
        let labels = self.node_labels();

        let check = NetworkReadiness {
            topology: self,
            expected_peer_counts: &expected_peer_counts,
            labels: &labels,
        };

        check.wait().await?;
        Ok(())
    }

    pub async fn wait_membership_ready(&self) -> Result<(), ReadinessError> {
        self.wait_membership_ready_for_session(SessionNumber::from(0u64))
            .await
    }

    pub async fn wait_membership_ready_for_session(
        &self,
        session: SessionNumber,
    ) -> Result<(), ReadinessError> {
        self.wait_membership_assignations(session, true).await
    }

    pub async fn wait_membership_empty_for_session(
        &self,
        session: SessionNumber,
    ) -> Result<(), ReadinessError> {
        self.wait_membership_assignations(session, false).await
    }

    async fn wait_membership_assignations(
        &self,
        session: SessionNumber,
        expect_non_empty: bool,
    ) -> Result<(), ReadinessError> {
        let total_nodes = self.validators.len() + self.executors.len();

        if total_nodes == 0 {
            return Ok(());
        }

        let labels = self.node_labels();
        let check = MembershipReadiness {
            topology: self,
            session,
            labels: &labels,
            expect_non_empty,
        };

        check.wait().await?;
        Ok(())
    }

    fn node_listen_ports(&self) -> Vec<u16> {
        self.validators
            .iter()
            .map(|node| node.config().network.backend.inner.port)
            .chain(
                self.executors
                    .iter()
                    .map(|node| node.config().network.backend.inner.port),
            )
            .collect()
    }

    fn node_initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.validators
            .iter()
            .map(|node| {
                node.config()
                    .network
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            })
            .chain(self.executors.iter().map(|node| {
                node.config()
                    .network
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            }))
            .collect()
    }

    fn node_labels(&self) -> Vec<String> {
        self.validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "validator#{idx}@{}",
                    node.config().network.backend.inner.port
                )
            })
            .chain(self.executors.iter().enumerate().map(|(idx, node)| {
                format!(
                    "executor#{idx}@{}",
                    node.config().network.backend.inner.port
                )
            }))
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum ReadinessError {
    #[error("{message}")]
    Timeout { message: String },
}

#[async_trait::async_trait]
trait ReadinessCheck<'a> {
    type Data: Send;

    async fn collect(&'a self) -> Self::Data;

    fn is_ready(&self, data: &Self::Data) -> bool;

    fn timeout_message(&self, data: Self::Data) -> String;

    fn poll_interval(&self) -> Duration {
        Duration::from_millis(200)
    }

    async fn wait(&'a self) -> Result<(), ReadinessError> {
        let timeout_duration = adjust_timeout(Duration::from_secs(60));
        let poll_interval = self.poll_interval();
        let mut data = self.collect().await;

        let wait_result = timeout(timeout_duration, async {
            loop {
                if self.is_ready(&data) {
                    return;
                }

                sleep(poll_interval).await;

                data = self.collect().await;
            }
        })
        .await;

        if wait_result.is_err() {
            let message = self.timeout_message(data);
            return Err(ReadinessError::Timeout { message });
        }

        Ok(())
    }
}

struct NetworkReadiness<'a> {
    topology: &'a Topology,
    expected_peer_counts: &'a [usize],
    labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for NetworkReadiness<'a> {
    type Data = Vec<Libp2pInfo>;

    async fn collect(&'a self) -> Self::Data {
        let (validator_infos, executor_infos) = tokio::join!(
            join_all(self.topology.validators.iter().map(Validator::network_info)),
            join_all(self.topology.executors.iter().map(Executor::network_info))
        );

        validator_infos.into_iter().chain(executor_infos).collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .enumerate()
            .all(|(idx, info)| info.n_peers >= self.expected_peer_counts[idx])
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(self.labels, data, self.expected_peer_counts);
        format!("timed out waiting for network readiness: {summary}")
    }
}

struct HttpNetworkReadiness<'a> {
    client: &'a Client,
    endpoints: &'a [Url],
    expected_peer_counts: &'a [usize],
    labels: &'a [String],
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for HttpNetworkReadiness<'a> {
    type Data = Vec<Libp2pInfo>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self
            .endpoints
            .iter()
            .map(|endpoint| fetch_network_info(self.client, endpoint));
        join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .enumerate()
            .all(|(idx, info)| info.n_peers >= self.expected_peer_counts[idx])
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let summary = build_timeout_summary(self.labels, data, self.expected_peer_counts);
        format!("timed out waiting for network readiness: {summary}")
    }
}

struct MembershipReadiness<'a> {
    topology: &'a Topology,
    session: SessionNumber,
    labels: &'a [String],
    expect_non_empty: bool,
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for MembershipReadiness<'a> {
    type Data = Vec<Result<MembershipResponse, reqwest::Error>>;

    async fn collect(&'a self) -> Self::Data {
        let (validator_responses, executor_responses) = tokio::join!(
            join_all(
                self.topology
                    .validators
                    .iter()
                    .map(|node| node.da_get_membership(self.session)),
            ),
            join_all(
                self.topology
                    .executors
                    .iter()
                    .map(|node| node.da_get_membership(self.session)),
            )
        );

        validator_responses
            .into_iter()
            .chain(executor_responses)
            .collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        self.assignation_statuses(data)
            .into_iter()
            .all(|ready| ready)
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let statuses = self.assignation_statuses(&data);
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_summary(self.labels, &statuses, description);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

impl MembershipReadiness<'_> {
    fn assignation_statuses(
        &self,
        responses: &[Result<MembershipResponse, reqwest::Error>],
    ) -> Vec<bool> {
        responses
            .iter()
            .map(|res| {
                res.as_ref()
                    .map(|resp| {
                        let is_non_empty = !resp.assignations.is_empty();
                        if self.expect_non_empty {
                            is_non_empty
                        } else {
                            !is_non_empty
                        }
                    })
                    .unwrap_or(false)
            })
            .collect()
    }
}

struct HttpMembershipReadiness<'a> {
    client: &'a Client,
    endpoints: &'a [Url],
    session: SessionNumber,
    labels: &'a [String],
    expect_non_empty: bool,
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for HttpMembershipReadiness<'a> {
    type Data = Vec<Result<MembershipResponse, reqwest::Error>>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self
            .endpoints
            .iter()
            .map(|endpoint| fetch_membership(self.client, endpoint, self.session));
        join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        assignation_statuses(data, self.expect_non_empty)
            .into_iter()
            .all(|ready| ready)
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let statuses = assignation_statuses(&data, self.expect_non_empty);
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_summary(self.labels, &statuses, description);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

fn build_timeout_summary(
    labels: &[String],
    infos: Vec<Libp2pInfo>,
    expected_counts: &[usize],
) -> String {
    infos
        .into_iter()
        .zip(expected_counts.iter())
        .zip(labels.iter())
        .map(|((info, expected), label)| {
            format!("{}: peers={}, expected={}", label, info.n_peers, expected)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_membership_summary(labels: &[String], statuses: &[bool], description: &str) -> String {
    statuses
        .iter()
        .zip(labels.iter())
        .map(|(ready, label)| {
            let status = if *ready { "ready" } else { "waiting" };
            format!("{label}: status={status}, expected {description}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn fetch_network_info(client: &Client, base: &Url) -> Libp2pInfo {
    let url = join_path(base, paths::NETWORK_INFO);
    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return log_network_warning(base, err, "failed to reach network info endpoint");
        }
    };

    let response = match response.error_for_status() {
        Ok(resp) => resp,
        Err(err) => {
            return log_network_warning(base, err, "network info endpoint returned error");
        }
    };

    match response.json::<Libp2pInfo>().await {
        Ok(info) => info,
        Err(err) => log_network_warning(base, err, "failed to decode network info response"),
    }
}

async fn fetch_membership(
    client: &Client,
    base: &Url,
    session: SessionNumber,
) -> Result<MembershipResponse, reqwest::Error> {
    let url = join_path(base, paths::DA_GET_MEMBERSHIP);
    client
        .post(url)
        .json(&session)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

fn log_network_warning(base: &Url, err: impl std::fmt::Display, message: &str) -> Libp2pInfo {
    warn!(target: "readiness", url = %base, error = %err, "{message}");
    empty_libp2p_info()
}

fn empty_libp2p_info() -> Libp2pInfo {
    Libp2pInfo {
        listen_addresses: Vec::with_capacity(0),
        n_peers: 0,
        n_connections: 0,
        n_pending_connections: 0,
    }
}

fn join_path(base: &Url, path: &str) -> Url {
    base.join(path.trim_start_matches('/'))
        .unwrap_or_else(|err| panic!("failed to join url {base} with path {path}: {err}"))
}

fn assignation_statuses(
    responses: &[Result<MembershipResponse, reqwest::Error>],
    expect_non_empty: bool,
) -> Vec<bool> {
    responses
        .iter()
        .map(|res| {
            res.as_ref()
                .map(|resp| {
                    let is_non_empty = !resp.assignations.is_empty();
                    if expect_non_empty {
                        is_non_empty
                    } else {
                        !is_non_empty
                    }
                })
                .unwrap_or(false)
        })
        .collect()
}

fn multiaddr_port(addr: &nomos_libp2p::Multiaddr) -> Option<u16> {
    for protocol in addr {
        match protocol {
            nomos_libp2p::Protocol::Udp(port) | nomos_libp2p::Protocol::Tcp(port) => {
                return Some(port);
            }
            _ => {}
        }
    }
    None
}

fn find_expected_peer_counts(
    listen_ports: &[u16],
    initial_peer_ports: &[HashSet<u16>],
) -> Vec<usize> {
    let mut expected: Vec<HashSet<usize>> = vec![HashSet::new(); initial_peer_ports.len()];

    for (idx, ports) in initial_peer_ports.iter().enumerate() {
        for port in ports {
            let Some(peer_idx) = listen_ports.iter().position(|p| p == port) else {
                continue;
            };
            if peer_idx == idx {
                continue;
            }

            expected[idx].insert(peer_idx);
            expected[peer_idx].insert(idx);
        }
    }

    expected.into_iter().map(|set| set.len()).collect()
}

#[must_use]
pub fn create_kms_configs(
    blend_configs: &[GeneralBlendConfig],
    da_configs: &[GeneralDaConfig],
) -> Vec<PreloadKMSBackendSettings> {
    da_configs
        .iter()
        .zip(blend_configs.iter())
        .map(|(da_conf, blend_conf)| PreloadKMSBackendSettings {
            keys: [
                (
                    hex::encode(blend_conf.signer.verifying_key().as_bytes()),
                    Key::Ed25519(Ed25519Key::new(blend_conf.signer.clone())),
                ),
                (
                    hex::encode(fr_to_bytes(
                        &blend_conf.secret_zk_key.to_public_key().into_inner(),
                    )),
                    Key::Zk(ZkKey::new(blend_conf.secret_zk_key.clone())),
                ),
                (
                    hex::encode(da_conf.signer.verifying_key().as_bytes()),
                    Key::Ed25519(Ed25519Key::new(da_conf.signer.clone())),
                ),
                (
                    hex::encode(fr_to_bytes(
                        &da_conf.secret_zk_key.to_public_key().into_inner(),
                    )),
                    Key::Zk(ZkKey::new(da_conf.secret_zk_key.clone())),
                ),
            ]
            .into(),
        })
        .collect()
}

fn resolve_ids(ids: Option<Vec<[u8; 32]>>, count: usize) -> Vec<[u8; 32]> {
    ids.map_or_else(
        || {
            let mut generated = vec![[0; 32]; count];
            for id in &mut generated {
                thread_rng().fill(id);
            }
            generated
        },
        |ids| {
            assert_eq!(
                ids.len(),
                count,
                "expected {count} ids but got {}",
                ids.len()
            );
            ids
        },
    )
}

fn resolve_ports(ports: Option<Vec<u16>>, count: usize, label: &str) -> Vec<u16> {
    let resolved = ports.unwrap_or_else(|| {
        iter::repeat_with(|| get_available_udp_port().unwrap())
            .take(count)
            .collect()
    });
    assert_eq!(
        resolved.len(),
        count,
        "expected {count} {label} ports but got {}",
        resolved.len()
    );
    resolved
}
