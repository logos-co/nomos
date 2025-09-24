use std::{
    collections::{BTreeSet, HashMap},
    net::Ipv4Addr,
    str::FromStr as _,
    time::Duration,
};

use nomos_core::sdp::{Locator, ServiceType};
use nomos_libp2p::{Multiaddr, ed25519, multiaddr};
use nomos_membership::{
    MembershipServiceSettings, backends::membership::MembershipBackendSettings,
};
use nomos_tracing_service::{LoggerLayer, MetricsLayer, TracingLayer, TracingSettings};
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use tests::{
    secret_key_to_provider_id,
    topology::configs::{
        GeneralConfig,
        api::GeneralApiConfig,
        blend::create_blend_configs,
        bootstrap::create_bootstrap_configs,
        consensus::{ConsensusParams, create_consensus_configs},
        da::{DaParams, create_da_configs},
        membership::GeneralMembershipConfig,
        network::{NetworkParams, create_network_configs},
        time::default_time_config,
        tracing::GeneralTracingConfig,
    },
};

const DEFAULT_LIBP2P_NETWORK_PORT: u16 = 3000;
const DEFAULT_DA_NETWORK_PORT: u16 = 3300;
const DEFAULT_BLEND_PORT: u16 = 3400;
const DEFAULT_API_PORT: u16 = 18080;

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
    pub network_port: u16,
    pub da_network_port: u16,
    pub blend_port: u16,
}

impl Host {
    #[must_use]
    pub const fn default_validator_from_ip(ip: Ipv4Addr, identifier: String) -> Self {
        Self {
            kind: HostKind::Validator,
            ip,
            identifier,
            network_port: DEFAULT_LIBP2P_NETWORK_PORT,
            da_network_port: DEFAULT_DA_NETWORK_PORT,
            blend_port: DEFAULT_BLEND_PORT,
        }
    }

    #[must_use]
    pub const fn default_executor_from_ip(ip: Ipv4Addr, identifier: String) -> Self {
        Self {
            kind: HostKind::Executor,
            ip,
            identifier,
            network_port: DEFAULT_LIBP2P_NETWORK_PORT,
            da_network_port: DEFAULT_DA_NETWORK_PORT,
            blend_port: DEFAULT_BLEND_PORT,
        }
    }
}

#[must_use]
pub fn create_node_configs(
    consensus_params: &ConsensusParams,
    da_params: &DaParams,
    tracing_settings: &TracingSettings,
    hosts: Vec<Host>,
) -> HashMap<Host, GeneralConfig> {
    let mut ids = vec![[0; 32]; consensus_params.n_participants];
    let mut ports = vec![];
    for id in &mut ids {
        thread_rng().fill(id);
        ports.push(get_available_udp_port().unwrap());
    }

    let consensus_configs = create_consensus_configs(&ids, consensus_params);
    let bootstrap_configs = create_bootstrap_configs(&ids, Duration::from_secs(60));
    let da_configs = create_da_configs(&ids, da_params, &ports);
    let network_configs = create_network_configs(&ids, &NetworkParams::default());
    let membership_configs = create_membership_configs(&ids, &hosts);
    let blend_configs = create_blend_configs(
        &ids,
        hosts
            .iter()
            .map(|h| h.blend_port)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let api_configs = ids
        .iter()
        .map(|_| GeneralApiConfig {
            address: format!("0.0.0.0:{DEFAULT_API_PORT}").parse().unwrap(),
        })
        .collect::<Vec<_>>();
    let mut configured_hosts = HashMap::new();

    // Rebuild DA address lists.
    let host_network_init_peers = update_network_init_peers(&hosts);

    for (i, host) in hosts.into_iter().enumerate() {
        let consensus_config = consensus_configs[i].clone();
        let api_config = api_configs[i].clone();

        // DA Libp2p network config.
        let mut da_config = da_configs[i].clone();
        da_config.listening_address = Multiaddr::from_str(&format!(
            "/ip4/0.0.0.0/udp/{}/quic-v1",
            host.da_network_port,
        ))
        .unwrap();
        if matches!(host.kind, HostKind::Validator) {
            da_config.policy_settings.min_dispersal_peers = 0;
        }

        // Libp2p network config.
        let mut network_config = network_configs[i].clone();
        network_config.swarm_config.host = Ipv4Addr::from_str("0.0.0.0").unwrap();
        network_config.swarm_config.port = host.network_port;
        network_config
            .initial_peers
            .clone_from(&host_network_init_peers);

        // Tracing config.
        let tracing_config =
            update_tracing_identifier(tracing_settings.clone(), host.identifier.clone());

        // Time config
        let time_config = default_time_config();

        configured_hosts.insert(
            host.clone(),
            GeneralConfig {
                consensus_config,
                bootstrapping_config: bootstrap_configs[i].clone(),
                da_config,
                network_config,
                blend_config: blend_configs[i].clone(),
                api_config,
                tracing_config,
                time_config,
                membership_config: membership_configs[i].clone(),
            },
        );
    }

    configured_hosts
}

fn update_network_init_peers(hosts: &[Host]) -> Vec<Multiaddr> {
    hosts
        .iter()
        .map(|h| multiaddr(h.ip, h.network_port))
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

#[must_use]
pub fn create_membership_configs(ids: &[[u8; 32]], hosts: &[Host]) -> Vec<GeneralMembershipConfig> {
    let mut providers = HashMap::new();

    for (i, id) in ids.iter().enumerate() {
        let mut node_key_bytes = *id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
            .expect("Failed to generate secret key from bytes");
        let provider_id = secret_key_to_provider_id(node_key.clone());

        let da_listening_address = Multiaddr::from_str(&format!(
            "/ip4/{}/udp/{}/quic-v1",
            hosts[i].ip, hosts[i].da_network_port,
        ))
        .expect("Failed to create multiaddr for DA");
        let blend_listening_address = Multiaddr::from_str(&format!(
            "/ip4/{}/udp/{}/quic-v1",
            hosts[i].ip, hosts[i].blend_port,
        ))
        .expect("Failed to create multiaddr for Blend");

        providers
            .entry(ServiceType::DataAvailability)
            .or_insert_with(HashMap::new)
            .insert(
                provider_id,
                BTreeSet::from([Locator::new(da_listening_address)]),
            );
        providers
            .entry(ServiceType::BlendNetwork)
            .or_insert_with(HashMap::new)
            .insert(
                provider_id,
                BTreeSet::from([Locator::new(blend_listening_address)]),
            );
    }

    let mock_backend_settings = MembershipBackendSettings {
        session_sizes: HashMap::from([
            (ServiceType::DataAvailability, 4),
            (ServiceType::BlendNetwork, 10),
        ]),
        session_zero_providers: providers,
    };

    let config = GeneralMembershipConfig {
        service_settings: MembershipServiceSettings {
            backend: mock_backend_settings,
        },
    };

    ids.iter().map(|_| config.clone()).collect()
}

#[cfg(test)]
mod cfgsync_tests {
    use std::{net::Ipv4Addr, num::NonZero, str::FromStr as _, time::Duration};

    use nomos_da_network_core::swarm::{
        DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
    };
    use nomos_libp2p::{Multiaddr, Protocol};
    use nomos_tracing_service::{
        ConsoleLayer, FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
    };
    use tests::topology::configs::{consensus::ConsensusParams, da::DaParams};
    use tracing::Level;

    use super::{Host, HostKind, create_node_configs};

    #[test]
    fn basic_ip_list() {
        let hosts = (0..10)
            .map(|i| Host {
                kind: HostKind::Validator,
                ip: Ipv4Addr::from_str(&format!("10.1.1.{i}")).unwrap(),
                identifier: "node".into(),
                network_port: 3000,
                da_network_port: 4044,
                blend_port: 5000,
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
        );

        for (host, config) in &configs {
            let network_port = config.network_config.swarm_config.port;
            let da_network_port = extract_port(&config.da_config.listening_address);
            let blend_port = extract_port(&config.blend_config.backend.listening_address);

            assert_eq!(network_port, host.network_port);
            assert_eq!(da_network_port, host.da_network_port);
            assert_eq!(blend_port, host.blend_port);
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
