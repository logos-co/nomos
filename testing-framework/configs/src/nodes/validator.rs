use std::{
    collections::HashSet,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use chain_leader::LeaderSettings;
use chain_service::{CryptarchiaSettings, OrphanConfig, StartingState, SyncConfig};
use cryptarchia_engine::time::SlotConfig;
use key_management_system::keys::{Key, ZkKey};
use nomos_blend_service::{
    core::settings::{CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings, ZkSettings},
    settings::TimingSettings,
};
use nomos_da_network_core::{
    protocols::sampling::SubnetsConfig, swarm::DAConnectionPolicySettings,
};
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig, api::http::ApiAdapterSettings,
    backends::libp2p::common::DaNetworkBackendSettings,
};
use nomos_da_sampling::{
    DaSamplingServiceSettings, backend::kzgrs::KzgrsSamplingBackendSettings,
    verifier::kzgrs::KzgrsDaVerifierSettings as SamplingVerifierSettings,
};
use nomos_da_verifier::{
    DaVerifierServiceSettings,
    backend::{kzgrs::KzgrsDaVerifierSettings, trigger::MempoolPublishTriggerConfig},
    storage::adapters::rocksdb::RocksAdapterSettings as VerifierStorageAdapterSettings,
};
use nomos_network::{backends::libp2p::Libp2pConfig, config::NetworkConfig};
use nomos_node::{
    Config as ValidatorConfig, RocksBackendSettings,
    api::backend::AxumBackendSettings as NodeAxumBackendSettings,
    config::{
        blend::{
            deployment::{self as blend_deployment},
            serde as blend_serde,
        },
        deployment::{CustomDeployment, Settings as NodeDeploymentSettings},
        mempool::MempoolConfig,
    },
};
use nomos_sdp::SdpSettings;
use nomos_time::{
    TimeServiceSettings,
    backends::{NtpTimeBackendSettings, ntp::async_client::NTPClientSettings},
};
use nomos_utils::math::NonNegativeF64;
use nomos_wallet::WalletServiceSettings;

use crate::{
    adjust_timeout,
    common::kms::key_id_for_preload_backend,
    topology::configs::{
        GeneralConfig, blend::GeneralBlendConfig as TopologyBlendConfig, wallet::WalletAccount,
    },
};

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Validator config wiring aggregates many service settings"
)]
pub fn create_validator_config(config: GeneralConfig) -> ValidatorConfig {
    let da_policy_settings = config.da_config.policy_settings;
    let (blend_user_config, deployment_settings) = build_blend_service_config(&config.blend_config);
    ValidatorConfig {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: config.network_config.swarm_config,
                initial_peers: config.network_config.initial_peers,
            },
        },
        blend: blend_user_config,
        deployment: deployment_settings,
        cryptarchia: CryptarchiaSettings {
            config: config.consensus_config.ledger_config.clone(),
            starting_state: StartingState::Genesis {
                genesis_tx: config.consensus_config.genesis_tx,
            },
            network_adapter_settings:
                chain_service::network::adapters::libp2p::LibP2pAdapterSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
            recovery_file: PathBuf::from("./recovery/cryptarchia.json"),
            bootstrap: chain_service::BootstrapConfig {
                prolonged_bootstrap_period: config.bootstrapping_config.prolonged_bootstrap_period,
                force_bootstrap: false,
                offline_grace_period: chain_service::OfflineGracePeriodConfig {
                    grace_period: Duration::from_secs(20 * 60),
                    state_recording_interval: Duration::from_secs(60),
                },
                ibd: chain_service::IbdConfig {
                    peers: HashSet::new(),
                    delay_before_new_download: Duration::from_secs(10),
                },
            },
            sync: SyncConfig {
                orphan: OrphanConfig {
                    max_orphan_cache_size: NonZeroUsize::new(5)
                        .expect("Max orphan cache size must be non-zero"),
                },
            },
        },
        cryptarchia_leader: LeaderSettings {
            transaction_selector_settings: (),
            config: config.consensus_config.ledger_config.clone(),
            leader_config: config.consensus_config.leader_config.clone(),
            blend_broadcast_settings:
                nomos_blend_service::core::network::libp2p::Libp2pBroadcastSettings {
                    topic: String::from(nomos_node::CONSENSUS_TOPIC),
                },
        },
        da_network: DaNetworkConfig {
            backend: DaNetworkBackendSettings {
                node_key: config.da_config.node_key,
                listening_address: config.da_config.listening_address,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: 0,
                    min_replication_peers: da_policy_settings.min_replication_peers,
                    max_dispersal_failures: da_policy_settings.max_dispersal_failures,
                    max_sampling_failures: da_policy_settings.max_sampling_failures,
                    max_replication_failures: da_policy_settings.max_replication_failures,
                    malicious_threshold: da_policy_settings.malicious_threshold,
                },
                monitor_settings: config.da_config.monitor_settings,
                balancer_interval: config.da_config.balancer_interval,
                redial_cooldown: config.da_config.redial_cooldown,
                replication_settings: config.da_config.replication_settings,
                subnets_settings: SubnetsConfig {
                    num_of_subnets: config.da_config.num_samples as usize,
                    shares_retry_limit: config.da_config.retry_shares_limit,
                    commitments_retry_limit: config.da_config.retry_commitments_limit,
                },
            },
            membership: config.da_config.membership.clone(),
            api_adapter_settings: ApiAdapterSettings {
                api_port: config.api_config.address.port(),
                is_secure: false,
            },
            subnet_refresh_interval: config.da_config.subnets_refresh_interval,
            subnet_threshold: config.da_config.num_subnets as usize,
            min_session_members: config.da_config.num_subnets as usize,
        },
        da_verifier: DaVerifierServiceSettings {
            share_verifier_settings: KzgrsDaVerifierSettings {
                global_params_path: config.da_config.global_params_path.clone(),
                domain_size: config.da_config.num_subnets as usize,
            },
            tx_verifier_settings: (),
            network_adapter_settings: (),
            storage_adapter_settings: VerifierStorageAdapterSettings {
                blob_storage_directory: "./".into(),
            },
            mempool_trigger_settings: MempoolPublishTriggerConfig {
                publish_threshold: NonNegativeF64::try_from(0.8).unwrap(),
                share_duration: Duration::from_secs(5),
                prune_duration: Duration::from_secs(30),
                prune_interval: Duration::from_secs(5),
            },
        },
        tracing: config.tracing_config.tracing_settings,
        http: nomos_api::ApiServiceSettings {
            backend_settings: NodeAxumBackendSettings {
                address: config.api_config.address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
        da_sampling: DaSamplingServiceSettings {
            sampling_settings: KzgrsSamplingBackendSettings {
                num_samples: config.da_config.num_samples,
                num_subnets: config.da_config.num_subnets,
                old_blobs_check_interval: config.da_config.old_blobs_check_interval,
                blobs_validity_duration: config.da_config.blobs_validity_duration,
            },
            share_verifier_settings: SamplingVerifierSettings {
                global_params_path: config.da_config.global_params_path,
                domain_size: config.da_config.num_subnets as usize,
            },
            commitments_wait_duration: Duration::from_secs(1),
            sdp_blob_trigger_sampling_delay: adjust_timeout(Duration::from_secs(5)),
        },
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        time: TimeServiceSettings {
            backend_settings: NtpTimeBackendSettings {
                ntp_server: config.time_config.ntp_server,
                ntp_client_settings: NTPClientSettings {
                    timeout: config.time_config.timeout,
                    listening_interface: config.time_config.interface,
                },
                update_interval: config.time_config.update_interval,
                slot_config: SlotConfig {
                    slot_duration: config.time_config.slot_duration,
                    chain_start_time: config.time_config.chain_start_time,
                },
                epoch_config: config.consensus_config.ledger_config.epoch_config,
                base_period_length: config.consensus_config.ledger_config.base_period_length(),
            },
        },
        mempool: MempoolConfig {
            pool_recovery_path: "./recovery/mempool.json".into(),
        },
        sdp: SdpSettings { declaration: None },
        wallet: WalletServiceSettings {
            known_keys: {
                let mut keys = HashSet::from_iter([config.consensus_config.leader_config.pk]);
                keys.extend(
                    config
                        .consensus_config
                        .wallet_accounts
                        .iter()
                        .map(WalletAccount::public_key),
                );
                keys
            },
        },
        key_management: config.kms_config,
        testing_http: nomos_api::ApiServiceSettings {
            backend_settings: NodeAxumBackendSettings {
                address: config.api_config.testing_http_address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
    }
}

fn build_blend_service_config(
    config: &TopologyBlendConfig,
) -> (blend_serde::Config, NodeDeploymentSettings) {
    let zk_key_id =
        key_id_for_preload_backend(&Key::from(ZkKey::new(config.secret_zk_key.clone())));

    let backend_core = &config.backend_core;
    let backend_edge = &config.backend_edge;

    let user = blend_serde::Config {
        common: blend_serde::common::Config {
            non_ephemeral_signing_key: config.private_key.clone(),
            recovery_path_prefix: PathBuf::from("./recovery/blend"),
        },
        core: blend_serde::core::Config {
            backend: blend_serde::core::BackendConfig {
                listening_address: backend_core.listening_address.clone(),
                core_peering_degree: backend_core.core_peering_degree.clone(),
                edge_node_connection_timeout: backend_core.edge_node_connection_timeout,
                max_edge_node_incoming_connections: backend_core.max_edge_node_incoming_connections,
                max_dial_attempts_per_peer: backend_core.max_dial_attempts_per_peer,
            },
            zk: ZkSettings {
                secret_key_kms_id: zk_key_id,
            },
        },
        edge: blend_serde::edge::Config {
            backend: blend_serde::edge::BackendConfig {
                max_dial_attempts_per_peer_per_message: backend_edge
                    .max_dial_attempts_per_peer_per_message,
                replication_factor: backend_edge.replication_factor,
            },
        },
    };

    let deployment_settings = blend_deployment::Settings {
        common: blend_deployment::CommonSettings {
            num_blend_layers: NonZeroU64::try_from(1).unwrap(),
            minimum_network_size: NonZeroU64::try_from(1).unwrap(),
            timing: TimingSettings {
                round_duration: Duration::from_secs(1),
                rounds_per_interval: NonZeroU64::try_from(30u64).unwrap(),
                rounds_per_session: NonZeroU64::try_from(648_000u64).unwrap(),
                rounds_per_observation_window: NonZeroU64::try_from(30u64).unwrap(),
                rounds_per_session_transition_period: NonZeroU64::try_from(30u64).unwrap(),
                epoch_transition_period_in_slots: NonZeroU64::try_from(2_600).unwrap(),
            },
            protocol_name: backend_core.protocol_name.clone(),
        },
        core: blend_deployment::CoreSettings {
            scheduler: SchedulerSettings {
                cover: CoverTrafficSettings {
                    intervals_for_safety_buffer: 100,
                    message_frequency_per_round: NonNegativeF64::try_from(1f64).unwrap(),
                },
                delayer: MessageDelayerSettings {
                    maximum_release_delay_in_rounds: NonZeroU64::try_from(3u64).unwrap(),
                },
            },
            minimum_messages_coefficient: backend_core.minimum_messages_coefficient,
            normalization_constant: backend_core.normalization_constant,
        },
    };

    let deployment = NodeDeploymentSettings::Custom(CustomDeployment {
        blend: deployment_settings,
    });

    (user, deployment)
}
