use std::{collections::HashMap, hash::BuildHasher, net::Ipv4Addr, str::FromStr as _};

use nomos_core::{
    mantle::GenesisTx as _,
    sdp::{Locator, ServiceType},
};
use nomos_libp2p::{Multiaddr, multiaddr};
use nomos_tracing_service::{LoggerLayer, MetricsLayer, TracingLayer, TracingSettings};
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use serde::{Deserialize, Serialize};
use tests::topology::{
    configs::{
        self, GeneralConfig,
        api::GeneralApiConfig,
        blend::GeneralBlendConfig,
        bootstrap::SHORT_PROLONGED_BOOTSTRAP_PERIOD,
        consensus::{
            ConsensusParams, GeneralConsensusConfig, ProviderInfo,
            create_genesis_tx_with_declarations,
        },
        da::{DaParams, GeneralDaConfig},
        network::{GeneralNetworkConfig, NetworkParams},
        time::default_time_config,
        tracing::GeneralTracingConfig,
        wallet::{WalletAccount, WalletConfig},
    },
    create_kms_configs,
};

const DEFAULT_LIBP2P_NETWORK_PORT: u16 = 3000;
const DEFAULT_DA_NETWORK_PORT: u16 = 3300;
const DEFAULT_BLEND_PORT: u16 = 3400;
const DEFAULT_API_PORT: u16 = 18080;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostPorts {
    pub network_port: u16,
    pub da_network_port: u16,
    pub blend_port: u16,
    pub api_port: u16,
    pub testing_http_port: u16,
}

impl Default for HostPorts {
    fn default() -> Self {
        Self {
            network_port: DEFAULT_LIBP2P_NETWORK_PORT,
            da_network_port: DEFAULT_DA_NETWORK_PORT,
            blend_port: DEFAULT_BLEND_PORT,
            api_port: DEFAULT_API_PORT,
            testing_http_port: DEFAULT_API_PORT,
        }
    }
}

#[derive(Eq, PartialEq, Hash, Clone)]
pub enum HostKind {
    Validator,
    Executor,
}

#[derive(Eq, PartialEq, Hash, Clone)]
pub struct Host {
    pub kind: HostKind,
    pub ip: Ipv4Addr,
    pub identifier: String,
    pub ports: HostPorts,
}

impl Host {
    #[must_use]
    pub fn validator_from_ip(ip: Ipv4Addr, identifier: String, ports: Option<HostPorts>) -> Self {
        Self::new(
            HostKind::Validator,
            ip,
            identifier,
            ports.unwrap_or_default(),
        )
    }

    #[must_use]
    pub fn executor_from_ip(ip: Ipv4Addr, identifier: String, ports: Option<HostPorts>) -> Self {
        Self::new(
            HostKind::Executor,
            ip,
            identifier,
            ports.unwrap_or_default(),
        )
    }

    #[must_use]
    pub fn default_validator_from_ip(ip: Ipv4Addr, identifier: String) -> Self {
        Self::new(HostKind::Validator, ip, identifier, HostPorts::default())
    }

    #[must_use]
    pub fn default_executor_from_ip(ip: Ipv4Addr, identifier: String) -> Self {
        Self::new(HostKind::Executor, ip, identifier, HostPorts::default())
    }

    #[expect(
        clippy::missing_const_for_fn,
        reason = "String and owned identifier not usable in const contexts"
    )]
    fn new(kind: HostKind, ip: Ipv4Addr, identifier: String, ports: HostPorts) -> Self {
        Self {
            kind,
            ip,
            identifier,
            ports,
        }
    }
}

#[must_use]
pub fn create_node_configs<S: BuildHasher>(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    hosts: Vec<Host>,
    prebuilt: Option<&HashMap<String, GeneralConfig, S>>,
) -> HashMap<Host, GeneralConfig> {
    if let Some(prebuilt) = prebuilt.filter(|map| !map.is_empty()) {
        let mut hosts = hosts;
        hosts.sort_by(|a, b| {
            host_sort_key(a)
                .cmp(&host_sort_key(b))
                .then_with(|| a.identifier.cmp(&b.identifier))
        });
        let host_network_init_peers = update_network_init_peers(&hosts);
        let mut entries = Vec::with_capacity(hosts.len());
        for host in hosts {
            let identifier = host.identifier.clone();
            let mut config = prebuilt
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing prebuilt config for {identifier}"))
                .clone();
            configure_da(&mut config.da_config, &host, da_params);
            configure_network(&mut config.network_config, &host, &host_network_init_peers);
            configure_blend(&mut config.blend_config, &host);
            config.api_config = GeneralApiConfig {
                address: format!("0.0.0.0:{}", host.ports.api_port).parse().unwrap(),
                testing_http_address: format!("0.0.0.0:{}", host.ports.testing_http_port)
                    .parse()
                    .unwrap(),
            };
            config.tracing_config = update_tracing_identifier(tracing_settings.clone(), identifier);
            config.time_config = default_time_config();
            entries.push((host, config));
        }
        recompute_genesis(&mut entries);
        return entries.into_iter().collect();
    }

    generate_random_configs(consensus_params, da_params, tracing_settings, hosts)
}

fn configure_da(config: &mut GeneralDaConfig, host: &Host, params: &DaParams) {
    config.listening_address = Multiaddr::from_str(&format!(
        "/ip4/0.0.0.0/udp/{}/quic-v1",
        host.ports.da_network_port
    ))
    .unwrap();
    config.policy_settings = params.policy_settings.clone();
    if matches!(host.kind, HostKind::Validator) {
        config.policy_settings.min_dispersal_peers = 0;
    }
    config.monitor_settings = params.monitor_settings.clone();
    config.balancer_interval = params.balancer_interval;
    config.redial_cooldown = params.redial_cooldown;
    config.replication_settings = params.replication_settings;
    config.subnets_refresh_interval = params.subnets_refresh_interval;
    config.retry_shares_limit = params.retry_shares_limit;
    config.retry_commitments_limit = params.retry_commitments_limit;
    config
        .global_params_path
        .clone_from(&params.global_params_path);
    config.num_samples = params.num_samples;
    config.num_subnets = params.num_subnets;
    config.old_blobs_check_interval = params.old_blobs_check_interval;
    config.blobs_validity_duration = params.blobs_validity_duration;
}

fn configure_network(config: &mut GeneralNetworkConfig, host: &Host, init_peers: &Vec<Multiaddr>) {
    config.swarm_config.host = Ipv4Addr::from_str("0.0.0.0").unwrap();
    config.swarm_config.port = host.ports.network_port;
    config.initial_peers.clone_from(init_peers);
    config.swarm_config.nat_config = nomos_libp2p::NatSettings::Static {
        external_address: Multiaddr::from_str(&format!(
            "/ip4/{}/udp/{}/quic-v1",
            host.ip, host.ports.network_port
        ))
        .unwrap(),
    };
}

fn configure_blend(config: &mut GeneralBlendConfig, host: &Host) {
    config.backend_core.listening_address = Multiaddr::from_str(&format!(
        "/ip4/0.0.0.0/udp/{}/quic-v1",
        host.ports.blend_port
    ))
    .unwrap();
}

const fn host_sort_key(host: &Host) -> u8 {
    match host.kind {
        HostKind::Validator => 0,
        HostKind::Executor => 1,
    }
}

fn recompute_genesis(entries: &mut [(Host, GeneralConfig)]) {
    if entries.is_empty() {
        return;
    }
    let hosts: Vec<_> = entries.iter().map(|(host, _)| host.clone()).collect();
    let consensus: Vec<_> = entries
        .iter()
        .map(|(_, config)| config.consensus_config.clone())
        .collect();
    let blend: Vec<_> = entries
        .iter()
        .map(|(_, config)| config.blend_config.clone())
        .collect();
    let da: Vec<_> = entries
        .iter()
        .map(|(_, config)| config.da_config.clone())
        .collect();

    let providers = create_providers(&hosts, &consensus, &blend, &da);
    let ledger_tx = consensus[0].genesis_tx.mantle_tx().ledger_tx.clone();
    let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
    for (_, config) in entries.iter_mut() {
        config.consensus_config.genesis_tx = genesis_tx.clone();
    }
}

fn create_providers(
    hosts: &[Host],
    consensus_configs: &[GeneralConsensusConfig],
    blend_configs: &[GeneralBlendConfig],
    da_configs: &[GeneralDaConfig],
) -> Vec<ProviderInfo> {
    let mut providers: Vec<_> = da_configs
        .iter()
        .enumerate()
        .map(|(i, da_conf)| ProviderInfo {
            service_type: ServiceType::DataAvailability,
            provider_sk: da_conf.signer.clone(),
            zk_sk: da_conf.secret_zk_key.clone(),
            locator: Locator(
                Multiaddr::from_str(&format!(
                    "/ip4/{}/udp/{}/quic-v1",
                    hosts[i].ip, hosts[i].ports.da_network_port
                ))
                .unwrap(),
            ),
            note: consensus_configs[0].da_notes[i].clone(),
        })
        .collect();
    providers.extend(blend_configs.iter().enumerate().map(|(i, blend_conf)| {
        ProviderInfo {
            service_type: ServiceType::BlendNetwork,
            provider_sk: blend_conf.signer.clone(),
            zk_sk: blend_conf.secret_zk_key.clone(),
            locator: Locator(
                Multiaddr::from_str(&format!(
                    "/ip4/{}/udp/{}/quic-v1",
                    hosts[i].ip, hosts[i].ports.blend_port
                ))
                .unwrap(),
            ),
            note: consensus_configs[0].blend_notes[i].clone(),
        }
    }));

    providers
}

fn generate_random_configs(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    hosts: Vec<Host>,
) -> HashMap<Host, GeneralConfig> {
    let mut ids = vec![[0; 32]; consensus_params.n_participants];
    let mut da_ports = vec![];
    let mut blend_ports = vec![];
    for id in &mut ids {
        thread_rng().fill(id);
        da_ports.push(get_available_udp_port().unwrap());
        blend_ports.push(get_available_udp_port().unwrap());
    }

    let mut consensus_configs = configs::consensus::create_consensus_configs(
        &ids,
        consensus_params,
        &WalletConfig::default(),
    );
    let bootstrap_configs =
        configs::bootstrap::create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
    let da_configs = configs::da::create_da_configs(&ids, da_params, &da_ports);
    let network_configs = configs::network::create_network_configs(&ids, &NetworkParams::default());
    let blend_configs = configs::blend::create_blend_configs(&ids, &blend_ports);
    let api_configs = ids
        .iter()
        .map(|_| {
            let address = format!("0.0.0.0:{DEFAULT_API_PORT}").parse().unwrap();
            GeneralApiConfig {
                address,
                testing_http_address: address,
            }
        })
        .collect::<Vec<_>>();

    let providers = create_providers(&hosts, &consensus_configs, &blend_configs, &da_configs);
    let ledger_tx = consensus_configs[0]
        .genesis_tx
        .mantle_tx()
        .ledger_tx
        .clone();
    let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
    for c in &mut consensus_configs {
        c.genesis_tx = genesis_tx.clone();
    }

    let kms_configs = create_kms_configs(&blend_configs, &da_configs, &[] as &[WalletAccount]);

    let host_network_init_peers = update_network_init_peers(&hosts);
    let mut configured_hosts = HashMap::new();

    for (i, host) in hosts.into_iter().enumerate() {
        let mut da_config = da_configs[i].clone();
        da_config.listening_address = Multiaddr::from_str(&format!(
            "/ip4/0.0.0.0/udp/{}/quic-v1",
            host.ports.da_network_port,
        ))
        .unwrap();
        if matches!(host.kind, HostKind::Validator) {
            da_config.policy_settings.min_dispersal_peers = 0;
        }

        let mut network_config = network_configs[i].clone();
        network_config.swarm_config.host = Ipv4Addr::from_str("0.0.0.0").unwrap();
        network_config.swarm_config.port = host.ports.network_port;
        network_config
            .initial_peers
            .clone_from(&host_network_init_peers);
        network_config.swarm_config.nat_config = nomos_libp2p::NatSettings::Static {
            external_address: Multiaddr::from_str(&format!(
                "/ip4/{}/udp/{}/quic-v1",
                host.ip, host.ports.network_port
            ))
            .unwrap(),
        };

        let mut blend_config = blend_configs[i].clone();
        blend_config.backend_core.listening_address = Multiaddr::from_str(&format!(
            "/ip4/0.0.0.0/udp/{}/quic-v1",
            host.ports.blend_port
        ))
        .unwrap();

        let tracing_config =
            update_tracing_identifier(tracing_settings.clone(), host.identifier.clone());
        let time_config = default_time_config();

        configured_hosts.insert(
            host,
            GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrap_configs[i].clone(),
                da_config,
                network_config,
                blend_config,
                api_config: api_configs[i].clone(),
                tracing_config,
                time_config,
                kms_config: kms_configs[i].clone(),
            },
        );
    }

    configured_hosts
}

fn update_network_init_peers(hosts: &[Host]) -> Vec<Multiaddr> {
    hosts
        .iter()
        .map(|h| multiaddr(h.ip, h.ports.network_port))
        .collect()
}

fn update_tracing_identifier(
    settings: TracingSettings,
    identifier: String,
) -> GeneralTracingConfig {
    GeneralTracingConfig {
        tracing_settings: TracingSettings {
            logger: match settings.logger {
                LoggerLayer::Loki(mut config) => {
                    config.host_identifier.clone_from(&identifier);
                    LoggerLayer::Loki(config)
                }
                other => other,
            },
            tracing: match settings.tracing {
                TracingLayer::Otlp(mut config) => {
                    config.service_name.clone_from(&identifier);
                    TracingLayer::Otlp(config)
                }
                other @ TracingLayer::None => other,
            },
            filter: settings.filter,
            metrics: match settings.metrics {
                MetricsLayer::Otlp(mut config) => {
                    config.host_identifier = identifier;
                    MetricsLayer::Otlp(config)
                }
                other @ MetricsLayer::None => other,
            },
            console: settings.console,
            level: settings.level,
        },
    }
}

#[cfg(test)]
mod cfgsync_tests {
    use std::{
        collections::HashMap, net::Ipv4Addr, num::NonZero, str::FromStr as _, time::Duration,
    };

    use nomos_da_network_core::swarm::{
        DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
    };
    use nomos_libp2p::{Multiaddr, Protocol};
    use nomos_tracing_service::{
        ConsoleLayer, FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
    };
    use tests::topology::configs::{GeneralConfig, consensus::ConsensusParams, da::DaParams};
    use tracing::Level;

    use super::{Host, HostPorts, create_node_configs};

    #[test]
    fn basic_ip_list() {
        let hosts = (0..10)
            .map(|i| {
                let ports = HostPorts {
                    network_port: 3000 + i,
                    da_network_port: 4044 + i,
                    blend_port: 5000 + i,
                    ..HostPorts::default()
                };
                Host::validator_from_ip(
                    Ipv4Addr::from_str(&format!("10.1.1.{i}")).unwrap(),
                    "node".into(),
                    Some(ports),
                )
            })
            .collect();

        let configs = create_node_configs(
            &ConsensusParams {
                n_participants: 10,
                security_param: NonZero::new(10).unwrap(),
                active_slot_coeff: 0.9,
            },
            &DaParams {
                subnetwork_size: 2,
                dispersal_factor: 1,
                num_samples: 1,
                num_subnets: 2,
                old_blobs_check_interval: Duration::from_secs(5),
                blobs_validity_duration: Duration::from_secs(u64::MAX),
                global_params_path: String::new(),
                policy_settings: DAConnectionPolicySettings::default(),
                monitor_settings: DAConnectionMonitorSettings::default(),
                balancer_interval: Duration::ZERO,
                redial_cooldown: Duration::ZERO,
                replication_settings: ReplicationConfig {
                    seen_message_cache_size: 0,
                    seen_message_ttl: Duration::ZERO,
                },
                subnets_refresh_interval: Duration::from_secs(1),
                retry_shares_limit: 1,
                retry_commitments_limit: 1,
            },
            &TracingSettings {
                logger: LoggerLayer::None,
                tracing: TracingLayer::None,
                filter: FilterLayer::None,
                metrics: MetricsLayer::None,
                console: ConsoleLayer::None,
                level: Level::DEBUG,
            },
            hosts,
            Option::<&HashMap<String, GeneralConfig>>::None,
        );

        for (host, config) in &configs {
            let network_port = config.network_config.swarm_config.port;
            let da_network_port = extract_port(&config.da_config.listening_address);
            let blend_port = extract_port(&config.blend_config.backend_core.listening_address);

            assert_eq!(network_port, host.ports.network_port);
            assert_eq!(da_network_port, host.ports.da_network_port);
            assert_eq!(blend_port, host.ports.blend_port);
        }
    }

    fn extract_port(multiaddr: &Multiaddr) -> u16 {
        multiaddr
            .iter()
            .find_map(|protocol| match protocol {
                Protocol::Udp(port) => Some(port),
                _ => None,
            })
            .unwrap()
    }
}
