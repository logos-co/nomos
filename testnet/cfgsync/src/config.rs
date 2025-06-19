use std::{
    collections::{BTreeSet, HashMap, HashSet},
    net::Ipv4Addr,
    str::FromStr as _,
};

use nomos_blend_scheduling::membership::Node;
use nomos_libp2p::{ed25519, multiaddr, Multiaddr, PeerId};
use nomos_membership::{
    backends::{mock::MockMembershipBackendSettings, MembershipBackendServiceSettings},
    MembershipServiceSettings,
};
use nomos_sdp_core::{Locator, ServiceType};
use nomos_tracing_service::{LoggerLayer, MetricsLayer, TracingLayer, TracingSettings};
use rand::{thread_rng, Rng as _};
use tests::{
    get_available_port, secret_key_to_provider_id,
    topology::configs::{
        api::GeneralApiConfig,
        blend::create_blend_configs,
        consensus::{create_consensus_configs, ConsensusParams},
        da::{create_da_configs, DaParams, GeneralDaConfig},
        membership::GeneralMembershipConfig,
        network::{create_network_configs, NetworkParams},
        time::default_time_config,
        tracing::GeneralTracingConfig,
        GeneralConfig,
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
        ports.push(get_available_port());
    }

    let consensus_configs = create_consensus_configs(&ids, consensus_params);
    let da_configs = create_da_configs(&ids, da_params, &ports);
    let network_configs = create_network_configs(&ids, &NetworkParams::default());
    let membership_configs = create_membership_configs(&ids, &hosts);
    let blend_configs = create_blend_configs(&ids);
    let api_configs = ids
        .iter()
        .map(|_| GeneralApiConfig {
            address: format!("0.0.0.0:{DEFAULT_API_PORT}").parse().unwrap(),
        })
        .collect::<Vec<_>>();
    let mut configured_hosts = HashMap::new();

    // Rebuild DA address lists.
    let host_network_init_peers = update_network_init_peers(&hosts);
    let host_da_peer_addresses = update_da_peer_addresses(hosts.clone(), da_configs.clone());
    let host_blend_membership =
        update_blend_membership(hosts.clone(), blend_configs[0].membership.clone());

    for (i, host) in hosts.into_iter().enumerate() {
        let consensus_config = consensus_configs[i].clone();
        let api_config = api_configs[i].clone();

        // DA Libp2p network config.
        let mut da_config = da_configs[i].clone();
        da_config.membership = da_config
            .membership
            .clone_with_different_addressbook(host_da_peer_addresses.clone());
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

        // Blend config.
        let mut blend_config = blend_configs[i].clone();
        blend_config.backend.listening_address =
            Multiaddr::from_str(&format!("/ip4/0.0.0.0/udp/{}/quic-v1", host.blend_port)).unwrap();
        blend_config.membership.clone_from(&host_blend_membership);

        // Tracing config.
        let tracing_config =
            update_tracing_identifier(tracing_settings.clone(), host.identifier.clone());

        // Time config
        let time_config = default_time_config();

        configured_hosts.insert(
            host.clone(),
            GeneralConfig {
                consensus_config,
                da_config,
                network_config,
                blend_config,
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

fn update_da_peer_addresses(
    hosts: Vec<Host>,
    da_configs: Vec<GeneralDaConfig>,
) -> HashMap<PeerId, Multiaddr> {
    da_configs
        .into_iter()
        .zip(hosts)
        .map(|(config, host)| {
            let new_multiaddr = Multiaddr::from_str(&format!(
                "/ip4/{}/udp/{}/quic-v1",
                host.ip, host.da_network_port,
            ))
            .unwrap();

            (config.peer_id, new_multiaddr)
        })
        .collect()
}

fn update_blend_membership(hosts: Vec<Host>, membership: Vec<Node<PeerId>>) -> Vec<Node<PeerId>> {
    membership
        .into_iter()
        .zip(hosts)
        .map(|(mut node, host)| {
            node.address =
                Multiaddr::from_str(&format!("/ip4/{}/udp/{}/quic-v1", host.ip, host.blend_port))
                    .unwrap();
            node
        })
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
            level: settings.level,
        },
    }
}

#[must_use]
pub fn create_membership_configs(ids: &[[u8; 32]], hosts: &[Host]) -> Vec<GeneralMembershipConfig> {
    let settings_per_service = HashMap::from([(
        ServiceType::DataAvailability,
        MembershipBackendServiceSettings {
            historical_block_delta: 8,
        },
    )]);

    let mut provider_ids = vec![];
    let mut locators = HashMap::new();

    for (i, id) in ids.iter().enumerate() {
        let mut node_key_bytes = *id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
            .expect("Failed to generate secret key from bytes");

        let provider_id = secret_key_to_provider_id(node_key.clone());
        provider_ids.push(provider_id);

        let listening_address = Multiaddr::from_str(&format!(
            "/ip4/{}/udp/{}/quic-v1",
            hosts[i].ip, hosts[i].da_network_port,
        ))
        .expect("Failed to create multiaddr");

        locators.insert(provider_id, Locator::new(listening_address));
    }

    let mut initial_membership = HashMap::new();
    let mut initial_locators_mapping = HashMap::new();
    let mut membership_entry = HashMap::new();
    let mut providers = HashSet::new();

    for provider_id in provider_ids {
        providers.insert(provider_id);
    }

    membership_entry.insert(ServiceType::DataAvailability, providers.clone());
    initial_membership.insert(0u64, membership_entry);

    for provider_id in providers {
        let mut locs = BTreeSet::new();
        if let Some(locator) = locators.get(&provider_id) {
            locs.insert(locator.clone());
        }
        initial_locators_mapping.insert(provider_id, locs);
    }

    let mock_backend_settings = MockMembershipBackendSettings {
        settings_per_service,
        initial_membership,
        initial_locators_mapping,
        latest_block_number: 0,
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

    use nomos_da_dispersal::backend::kzgrs::MempoolPublishStrategy;
    use nomos_da_network_core::swarm::{
        DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
    };
    use nomos_libp2p::{ed25519, libp2p, Multiaddr, PeerId, Protocol};
    use nomos_tracing_service::{
        FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
    };
    use subnetworks_assignations::MembershipHandler as _;
    use tests::topology::configs::{consensus::ConsensusParams, da::DaParams, GeneralConfig};
    use tracing::Level;

    use super::{create_node_configs, Host, HostKind};
    use crate::tests::extract_ip;

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
                mempool_strategy: MempoolPublishStrategy::Immediately,
                policy_settings: DAConnectionPolicySettings::default(),
                monitor_settings: DAConnectionMonitorSettings::default(),
                balancer_interval: Duration::ZERO,
                redial_cooldown: Duration::ZERO,
                replication_settings: ReplicationConfig {
                    seen_message_cache_size: 0,
                    seen_message_ttl: Duration::ZERO,
                },
            },
            &TracingSettings {
                logger: LoggerLayer::None,
                tracing: TracingLayer::None,
                filter: FilterLayer::None,
                metrics: MetricsLayer::None,
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

            check_da_membership(host.ip, config);
        }
    }

    pub fn check_da_membership(my_ip: Ipv4Addr, config: &GeneralConfig) {
        let key = libp2p::identity::Keypair::from(ed25519::Keypair::from(
            config.da_config.node_key.clone(),
        ));
        let my_peer_id = PeerId::from_public_key(&key.public());
        let my_multiaddr = config
            .da_config
            .membership
            .get_address(&my_peer_id)
            .unwrap();
        let my_multiaddr_ip = extract_ip(&my_multiaddr).unwrap();
        assert_eq!(
            my_ip, my_multiaddr_ip,
            "DA membership ip doesn't match host ip"
        );
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
