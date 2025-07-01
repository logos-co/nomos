use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs as _},
    path::{Path, PathBuf},
};

use clap::{builder::OsStr, Parser, ValueEnum};
use color_eyre::eyre::{eyre, Result};
use hex::FromHex as _;
use nomos_core::mantle::{Note, Utxo};
use nomos_libp2p::{ed25519::SecretKey, Multiaddr};
use nomos_network::backends::libp2p::Libp2p as NetworkBackend;
use nomos_tracing::logging::{gelf::GelfConfig, local::FileConfig};
use nomos_tracing_service::{LoggerLayer, Tracing};
use overwatch::services::ServiceData;
use serde::{Deserialize, Serialize};
use tracing::Level;

use crate::{
    config::mempool::MempoolConfig,
    generic_services::{MembershipService, SdpService},
    ApiService, BlendService, CryptarchiaService, DaIndexerService, DaNetworkService,
    DaSamplingService, DaVerifierService, NetworkService, RuntimeServiceId, StorageService,
    TimeService,
};

pub mod mempool;
#[cfg(test)]
mod tests;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    /// Path for a yaml-encoded network config file
    config: PathBuf,
    /// Dry-run flag. If active, the binary will try to deserialize the config
    /// file and then exit.
    #[clap(long = "check-config", action)]
    check_config_only: bool,
    /// Overrides log config.
    #[clap(flatten)]
    log: LogArgs,
    /// Overrides network config.
    #[clap(flatten)]
    network: NetworkArgs,
    /// Overrides blend config.
    #[clap(flatten)]
    blend: BlendArgs,
    /// Overrides http config.
    #[clap(flatten)]
    http: HttpArgs,
    #[clap(flatten)]
    cryptarchia: CryptarchiaArgs,
    #[clap(flatten)]
    da: DaArgs,
}

impl CliArgs {
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config
    }

    /// If flags the blend service group to start if either all service groups
    /// are flagged to start or the blend service group is.
    #[must_use]
    pub const fn dry_run(&self) -> bool {
        self.check_config_only
    }

    #[must_use]
    pub const fn must_blend_service_group_start(&self) -> bool {
        self.must_all_service_groups_start() || self.blend.start_blend_at_boot
    }

    /// If flags the DA service group to start if either all service groups are
    /// flagged to start or the DA service group is.
    #[must_use]
    pub const fn must_da_service_group_start(&self) -> bool {
        self.must_all_service_groups_start() || self.da.start_da_at_boot
    }

    /// If no "start" flag is explicitly set for any service group, then all
    /// service groups are flagged to start.
    const fn must_all_service_groups_start(&self) -> bool {
        !self.blend.start_blend_at_boot && !self.da.start_da_at_boot
    }
}

#[derive(ValueEnum, Clone, Debug, Default)]
pub enum LoggerLayerType {
    Gelf,
    File,
    #[default]
    Stdout,
    Stderr,
}

impl From<LoggerLayerType> for OsStr {
    fn from(value: LoggerLayerType) -> Self {
        match value {
            LoggerLayerType::Gelf => "Gelf".into(),
            LoggerLayerType::File => "File".into(),
            LoggerLayerType::Stderr => "Stderr".into(),
            LoggerLayerType::Stdout => "Stdout".into(),
        }
    }
}

#[derive(Parser, Debug, Clone)]
pub struct LogArgs {
    /// Address for the Gelf backend
    #[clap(
        long = "log-addr",
        env = "LOG_ADDR",
        required_if_eq("backend", LoggerLayerType::Gelf)
    )]
    log_addr: Option<String>,

    /// Directory for the File backend
    #[clap(
        long = "log-dir",
        env = "LOG_DIR",
        required_if_eq("backend", LoggerLayerType::File)
    )]
    directory: Option<PathBuf>,

    /// Prefix for the File backend
    #[clap(
        long = "log-path",
        env = "LOG_PATH",
        required_if_eq("backend", LoggerLayerType::File)
    )]
    prefix: Option<PathBuf>,

    /// Backend type
    #[clap(long = "log-backend", env = "LOG_BACKEND", value_enum)]
    backend: Option<LoggerLayerType>,

    #[clap(long = "log-level", env = "LOG_LEVEL")]
    level: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct NetworkArgs {
    #[clap(long = "net-host", env = "NET_HOST")]
    host: Option<IpAddr>,

    #[clap(long = "net-port", env = "NET_PORT")]
    port: Option<usize>,

    // TODO: Use either the raw bytes or the key type directly to delegate error handling to clap
    #[clap(long = "net-node-key", env = "NET_NODE_KEY")]
    node_key: Option<String>,

    #[clap(long = "net-initial-peers", env = "NET_INITIAL_PEERS", num_args = 1.., value_delimiter = ',')]
    pub initial_peers: Option<Vec<Multiaddr>>,
}

#[derive(Parser, Debug, Clone)]
pub struct BlendArgs {
    #[clap(long = "blend-addr", env = "BLEND_ADDR")]
    blend_addr: Option<Multiaddr>,

    // TODO: Use either the raw bytes or the key type directly to delegate error handling to clap
    #[clap(long = "blend-node-key", env = "BLEND_NODE_KEY")]
    blend_node_key: Option<String>,

    #[clap(long = "blend-num-blend-layers", env = "BLEND_NUM_BLEND_LAYERS")]
    blend_num_blend_layers: Option<usize>,
    #[clap(long = "blend-service-group", action)]
    start_blend_at_boot: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct HttpArgs {
    #[clap(long = "http-host", env = "HTTP_HOST")]
    pub http_addr: Option<SocketAddr>,

    #[clap(long = "http-cors-origin", env = "HTTP_CORS_ORIGIN")]
    pub cors_origins: Option<Vec<String>>,
}

#[derive(Parser, Debug, Clone)]
pub struct CryptarchiaArgs {
    #[clap(
        long = "consensus-utxo-sk",
        env = "CONSENSUS_UTXO_SK",
        requires = "value"
    )]
    pub secret_key: Option<String>,

    #[clap(
        long = "consensus-utxo-value",
        env = "CONSENSUS_UTXO_VALUE",
        requires = "secret_key"
    )]
    value: Option<u64>,

    #[clap(
        long = "consensus-utxo-txhash",
        env = "CONSENSUS_UTXO_TXHASH",
        requires = "value"
    )]
    tx_hash: Option<String>,

    #[clap(
        long = "consensus-utxo-output-index",
        env = "CONSENSUS_UTXO_OUTPUT_INDEX",
        requires = "value"
    )]
    output_index: Option<usize>,
}

#[derive(Parser, Debug, Clone)]
pub struct TimeArgs {
    #[clap(long = "consensus-chain-start", env = "CONSENSUS_CHAIN_START")]
    chain_start_time: Option<i64>,

    #[clap(long = "consensus-slot-duration", env = "CONSENSUS_SLOT_DURATION")]
    slot_duration: Option<u64>,
}

#[derive(Parser, Debug, Clone)]
pub struct DaArgs {
    #[clap(long = "da-service-group", action)]
    start_da_at_boot: bool,
}

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Config {
    pub tracing: <Tracing<RuntimeServiceId> as ServiceData>::Settings,
    pub network: <NetworkService as ServiceData>::Settings,
    pub blend: <BlendService as ServiceData>::Settings,
    pub da_network: <DaNetworkService as ServiceData>::Settings,
    pub da_indexer: <DaIndexerService as ServiceData>::Settings,
    pub da_verifier: <DaVerifierService as ServiceData>::Settings,
    pub membership: <MembershipService<RuntimeServiceId> as ServiceData>::Settings,
    pub sdp: <SdpService<RuntimeServiceId> as ServiceData>::Settings,
    pub da_sampling: <DaSamplingService as ServiceData>::Settings,
    pub http: <ApiService as ServiceData>::Settings,
    pub cryptarchia: <CryptarchiaService as ServiceData>::Settings,
    pub time: <TimeService as ServiceData>::Settings,
    pub storage: <StorageService as ServiceData>::Settings,
    pub mempool: MempoolConfig,

    #[cfg(feature = "testing")]
    pub testing_http: <ApiService as ServiceData>::Settings,
}

impl Config {
    pub fn update_from_args(mut self, args: CliArgs) -> Result<Self> {
        let CliArgs {
            log: log_args,
            http: http_args,
            network: network_args,
            blend: blend_args,
            cryptarchia: cryptarchia_args,
            ..
        } = args;
        update_tracing(&mut self.tracing, log_args)?;
        update_network::<RuntimeServiceId>(&mut self.network, network_args)?;
        update_blend(&mut self.blend, blend_args)?;
        update_http(&mut self.http, http_args)?;
        update_cryptarchia_consensus(&mut self.cryptarchia, cryptarchia_args)?;
        Ok(self)
    }
}

pub fn update_tracing(
    tracing: &mut <Tracing<RuntimeServiceId> as ServiceData>::Settings,
    tracing_args: LogArgs,
) -> Result<()> {
    let LogArgs {
        backend,
        log_addr: addr,
        directory,
        prefix,
        level,
    } = tracing_args;

    // Override the file config with the one from env variables.
    if let Some(backend) = backend {
        tracing.logger = match backend {
            LoggerLayerType::Gelf => LoggerLayer::Gelf(GelfConfig {
                addr: addr
                    .ok_or_else(|| eyre!("Gelf backend requires an address."))?
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| eyre!("Invalid gelf address"))?,
            }),
            LoggerLayerType::File => LoggerLayer::File(FileConfig {
                directory: directory.ok_or_else(|| eyre!("File backend requires a directory."))?,
                prefix,
            }),
            LoggerLayerType::Stdout => LoggerLayer::Stdout,
            LoggerLayerType::Stderr => LoggerLayer::Stderr,
        }
    }

    if let Some(level_str) = level {
        tracing.level = match level_str.as_str() {
            "DEBUG" => Level::DEBUG,
            "INFO" => Level::INFO,
            "ERROR" => Level::ERROR,
            "WARN" => Level::WARN,
            _ => return Err(eyre!("Invalid log level provided.")),
        };
    }
    Ok(())
}

pub fn update_network<RuntimeServiceId>(
    network: &mut <nomos_network::NetworkService<NetworkBackend, RuntimeServiceId> as ServiceData>::Settings,
    network_args: NetworkArgs,
) -> Result<()> {
    let NetworkArgs {
        host,
        port,
        node_key,
        initial_peers,
    } = network_args;

    if let Some(IpAddr::V4(h)) = host {
        network.backend.inner.host = h;
    } else if host.is_some() {
        return Err(eyre!("Unsupported ip version"));
    }

    if let Some(port) = port {
        network.backend.inner.port = port as u16;
    }

    if let Some(node_key) = node_key {
        let mut key_bytes = hex::decode(node_key)?;
        network.backend.inner.node_key = SecretKey::try_from_bytes(key_bytes.as_mut_slice())?;
    }

    if let Some(peers) = initial_peers {
        network.backend.initial_peers = peers;
    }

    Ok(())
}

pub fn update_blend(
    blend: &mut <BlendService as ServiceData>::Settings,
    blend_args: BlendArgs,
) -> Result<()> {
    let BlendArgs {
        blend_addr,
        blend_node_key,
        blend_num_blend_layers,
        ..
    } = blend_args;

    if let Some(addr) = blend_addr {
        blend.backend.listening_address = addr;
    }

    if let Some(node_key) = blend_node_key {
        let mut key_bytes = hex::decode(node_key)?;
        blend.backend.node_key = SecretKey::try_from_bytes(key_bytes.as_mut_slice())?;
    }

    if let Some(num_blend_layers) = blend_num_blend_layers {
        blend.crypto.num_blend_layers = num_blend_layers as u64;
    }

    Ok(())
}

pub fn update_http(
    http: &mut <ApiService as ServiceData>::Settings,
    http_args: HttpArgs,
) -> Result<()> {
    let HttpArgs {
        http_addr,
        cors_origins,
    } = http_args;

    if let Some(addr) = http_addr {
        http.backend_settings.address = addr;
    }

    if let Some(cors) = cors_origins {
        http.backend_settings.cors_origins = cors;
    }

    Ok(())
}

pub fn update_cryptarchia_consensus(
    cryptarchia: &mut <CryptarchiaService as ServiceData>::Settings,
    consensus_args: CryptarchiaArgs,
) -> Result<()> {
    let CryptarchiaArgs {
        secret_key,
        value,
        tx_hash,
        output_index,
    } = consensus_args;

    let (Some(secret_key), Some(value), Some(tx_hash), Some(output_index)) =
        (secret_key, value, tx_hash, output_index)
    else {
        return Ok(());
    };

    let sk = nomos_core::mantle::keys::SecretKey::from(<[u8; 16]>::from_hex(secret_key)?);
    cryptarchia.leader_config.sk = sk;

    let pk = sk.to_public_key();

    let tx_hash = <[u8; 32]>::from_hex(tx_hash)?;
    cryptarchia.leader_config.utxos.push(Utxo {
        tx_hash: tx_hash.into(),
        output_index,
        note: Note { value, pk },
    });

    Ok(())
}

pub fn update_time(
    time: &mut <TimeService as ServiceData>::Settings,
    time_args: &TimeArgs,
) -> Result<()> {
    let TimeArgs {
        chain_start_time,
        slot_duration,
    } = *time_args;
    if let Some(start_time) = chain_start_time {
        time.backend_settings.slot_config.chain_start_time =
            time::OffsetDateTime::from_unix_timestamp(start_time)?;
    }

    if let Some(duration) = slot_duration {
        time.backend_settings.slot_config.slot_duration = std::time::Duration::from_secs(duration);
    }
    Ok(())
}
