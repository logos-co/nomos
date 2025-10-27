use std::{
    collections::HashSet,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use chain_leader::LeaderSettings;
use chain_service::{CryptarchiaSettings, OrphanConfig, StartingState, SyncConfig};
use cryptarchia_engine::time::SlotConfig;
use nomos_api::ApiServiceSettings;
use nomos_blend_scheduling::message_blend::crypto::SessionCryptographicProcessorSettings;
use nomos_blend_service::{
    core::settings::{
        CoverTrafficSettingsExt, MessageDelayerSettingsExt, SchedulerSettingsExt, ZkSettings,
    },
    settings::TimingSettings,
};
use nomos_da_dispersal::{
    DispersalServiceSettings,
    backend::kzgrs::{DispersalKZGRSBackendSettings, EncoderSettings},
};
use nomos_da_network_core::{
    protocols::sampling::SubnetsConfig, swarm::DAConnectionPolicySettings,
};
use nomos_da_network_service::{
    NetworkConfig as DaNetworkConfig,
    api::http::ApiAdapterSettings,
    backends::libp2p::{
        common::DaNetworkBackendSettings, executor::DaNetworkExecutorBackendSettings,
    },
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
use nomos_executor::{
    api::backend::AxumBackendSettings as ExecutorAxumBackendSettings,
    config::Config as ExecutorConfig,
};
use nomos_network::{backends::libp2p::Libp2pConfig, config::NetworkConfig};
use nomos_node::{
    Config as ValidatorConfig, RocksBackendSettings,
    api::backend::AxumBackendSettings as NodeAxumBackendSettings,
    config::{blend::BlendConfig, mempool::MempoolConfig},
};
use nomos_sdp::SdpSettings;
use nomos_time::{
    TimeServiceSettings,
    backends::{NtpTimeBackendSettings, ntp::async_client::NTPClientSettings},
};
use nomos_utils::math::NonNegativeF64;
use nomos_wallet::WalletServiceSettings;

use crate::{adjust_timeout, topology::configs::GeneralConfig};

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Validator config wiring aggregates many service settings"
)]
pub fn create_validator_config(config: GeneralConfig) -> ValidatorConfig {
    let testing_http_address = config.api_config.testing_http_address;
    let da_policy_settings = config.da_config.policy_settings;
    ValidatorConfig {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: config.network_config.swarm_config,
                initial_peers: config.network_config.initial_peers,
            },
        },
        blend: BlendConfig::new(nomos_blend_service::core::settings::BlendConfig {
            backend: config.blend_config.backend,
            crypto: SessionCryptographicProcessorSettings {
                non_ephemeral_signing_key: config.blend_config.private_key.clone(),
                num_blend_layers: 1,
            },
            time: TimingSettings {
                round_duration: Duration::from_secs(1),
                rounds_per_interval: NonZeroU64::try_from(30u64)
                    .expect("Rounds per interval cannot be zero."),
                // (21,600 blocks * 30s per block) / 1s per round = 648,000 rounds
                rounds_per_session: NonZeroU64::try_from(648_000u64)
                    .expect("Rounds per session cannot be zero."),
                rounds_per_observation_window: NonZeroU64::try_from(30u64)
                    .expect("Rounds per observation window cannot be zero."),
                rounds_per_session_transition_period: NonZeroU64::try_from(30u64)
                    .expect("Rounds per session transition period cannot be zero."),
                epoch_transition_period_in_slots: NonZeroU64::try_from(2_600)
                    .expect("Epoch transition period in slots cannot be zero."),
            },
            scheduler: SchedulerSettingsExt {
                cover: CoverTrafficSettingsExt {
                    intervals_for_safety_buffer: 100,
                    message_frequency_per_round: NonNegativeF64::try_from(1f64)
                        .expect("Message frequency per round cannot be negative."),
                    redundancy_parameter: 0,
                },
                delayer: MessageDelayerSettingsExt {
                    maximum_release_delay_in_rounds: NonZeroU64::try_from(3u64)
                        .expect("Maximum release delay between rounds cannot be zero."),
                },
            },
            zk: ZkSettings {
                sk: config.blend_config.secret_zk_key.clone(),
            },
            minimum_network_size: 1
                .try_into()
                .expect("Minimum Blend network size cannot be zero."),
        }),
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
        http: ApiServiceSettings {
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
        },
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        // TODO from
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
            trigger_sampling_delay: adjust_timeout(Duration::from_secs(5)),
        },
        membership: config.membership_config.service_settings,
        sdp: SdpSettings {
            declaration_id: None,
        },
        wallet: WalletServiceSettings {
            known_keys: HashSet::from_iter([config.consensus_config.leader_config.pk]),
        },
        testing_http: ApiServiceSettings {
            backend_settings: NodeAxumBackendSettings {
                address: testing_http_address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
    }
}

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Executor config wiring aggregates many service settings"
)]
pub fn create_executor_config(config: GeneralConfig) -> ExecutorConfig {
    let testing_http_address = config.api_config.testing_http_address;

    ExecutorConfig {
        network: NetworkConfig {
            backend: Libp2pConfig {
                inner: config.network_config.swarm_config,
                initial_peers: config.network_config.initial_peers,
            },
        },
        blend: BlendConfig::new(nomos_blend_service::core::settings::BlendConfig {
            backend: config.blend_config.backend,
            crypto: SessionCryptographicProcessorSettings {
                non_ephemeral_signing_key: config.blend_config.private_key.clone(),
                num_blend_layers: 1,
            },
            time: TimingSettings {
                round_duration: Duration::from_secs(1),
                rounds_per_interval: NonZeroU64::try_from(30u64)
                    .expect("Rounds per interval cannot be zero."),
                // (21,600 blocks * 30s per block) / 1s per round = 648,000 rounds
                rounds_per_session: NonZeroU64::try_from(648_000u64)
                    .expect("Rounds per session cannot be zero."),
                rounds_per_observation_window: NonZeroU64::try_from(30u64)
                    .expect("Rounds per observation window cannot be zero."),
                rounds_per_session_transition_period: NonZeroU64::try_from(30u64)
                    .expect("Rounds per session transition period cannot be zero."),
                epoch_transition_period_in_slots: NonZeroU64::try_from(2_600)
                    .expect("Epoch transition period in slots cannot be zero."),
            },
            scheduler: SchedulerSettingsExt {
                cover: CoverTrafficSettingsExt {
                    intervals_for_safety_buffer: 100,
                    message_frequency_per_round: NonNegativeF64::try_from(1f64)
                        .expect("Message frequency per round cannot be negative."),
                    redundancy_parameter: 0,
                },
                delayer: MessageDelayerSettingsExt {
                    maximum_release_delay_in_rounds: NonZeroU64::try_from(3u64)
                        .expect("Maximum release delay between rounds cannot be zero."),
                },
            },
            zk: ZkSettings {
                sk: config.blend_config.secret_zk_key.clone(),
            },
            minimum_network_size: 1
                .try_into()
                .expect("Minimum Blend network size cannot be zero."),
        }),
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
                prolonged_bootstrap_period: Duration::from_secs(3),
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
            backend: DaNetworkExecutorBackendSettings {
                validator_settings: DaNetworkBackendSettings {
                    node_key: config.da_config.node_key,
                    listening_address: config.da_config.listening_address,
                    policy_settings: config.da_config.policy_settings,
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
                num_subnets: config.da_config.num_subnets,
            },
            membership: config.da_config.membership.clone(),
            api_adapter_settings: ApiAdapterSettings {
                api_port: config.api_config.address.port(),
                is_secure: false,
            },
            subnet_refresh_interval: config.da_config.subnets_refresh_interval,
            subnet_threshold: config.da_config.num_subnets as usize,
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
        http: ApiServiceSettings {
            backend_settings: ExecutorAxumBackendSettings {
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
                global_params_path: config.da_config.global_params_path.clone(),
                domain_size: config.da_config.num_subnets as usize,
            },
            commitments_wait_duration: Duration::from_secs(1),
        },
        storage: RocksBackendSettings {
            db_path: "./db".into(),
            read_only: false,
            column_family: Some("blocks".into()),
        },
        da_dispersal: DispersalServiceSettings {
            backend: DispersalKZGRSBackendSettings {
                encoder_settings: EncoderSettings {
                    num_columns: config.da_config.num_subnets as usize,
                    with_cache: false,
                    global_params_path: config.da_config.global_params_path,
                },
                dispersal_timeout: Duration::from_secs(20),
                retry_cooldown: Duration::from_secs(3),
                retry_limit: 2,
            },
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
            trigger_sampling_delay: adjust_timeout(Duration::from_secs(5)),
        },
        membership: config.membership_config.service_settings,
        sdp: SdpSettings {
            declaration_id: None,
        },
        wallet: WalletServiceSettings {
            known_keys: HashSet::from_iter([config.consensus_config.leader_config.pk]),
        },

        testing_http: ApiServiceSettings {
            backend_settings: ExecutorAxumBackendSettings {
                address: testing_http_address,
                rate_limit_per_second: 10000,
                rate_limit_burst: 10000,
                max_concurrent_requests: 1000,
                ..Default::default()
            },
        },
    }
}
