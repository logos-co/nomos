use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs as _},
    path::PathBuf,
};

use cl::{Nonce, NoteWitness, NullifierSecret};
use clap::{Parser, ValueEnum};
use color_eyre::eyre::{eyre, Result};
use hex::FromHex as _;
use nomos_core::{proofs::covenant::CovenantProof, staking::NMO_UNIT};
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

#[derive(ValueEnum, Clone, Debug, Default)]
pub enum LoggerLayerType {
    Gelf,
    File,
    #[default]
    Stdout,
    Stderr,
}

#[derive(Parser, Debug, Clone)]
pub struct LogArgs {
    /// Address for the Gelf backend
    #[clap(long = "log-addr", env = "LOG_ADDR", required_if_eq("backend", "Gelf"))]
    log_addr: Option<String>,

    /// Directory for the File backend
    #[clap(long = "log-dir", env = "LOG_DIR", required_if_eq("backend", "File"))]
    directory: Option<PathBuf>,

    /// Prefix for the File backend
    #[clap(long = "log-path", env = "LOG_PATH", required_if_eq("backend", "File"))]
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
        long = "consensus-note-sk",
        env = "CONSENSUS_NOTE_SK",
        requires("note_value")
    )]
    note_secret_key: Option<String>,

    #[clap(
        long = "consensus-note-value",
        env = "CONSENSUS_NOTE_VALUE",
        requires("note_secret_key")
    )]
    note_value: Option<u32>,

    #[clap(
        long = "consensus-note-nonce",
        env = "CONSENSUS_NOTE_NONCE",
        requires("note_value")
    )]
    note_nonce: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct TimeArgs {
    #[clap(long = "consensus-chain-start", env = "CONSENSUS_CHAIN_START")]
    chain_start_time: Option<i64>,

    #[clap(long = "consensus-slot-duration", env = "CONSENSUS_SLOT_DURATION")]
    slot_duration: Option<u64>,
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
}

impl Config {
    pub fn update_from_args(
        mut self,
        log_args: LogArgs,
        network_args: NetworkArgs,
        blend_args: BlendArgs,
        http_args: HttpArgs,
        cryptarchia_args: CryptarchiaArgs,
    ) -> Result<Self> {
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
    } = blend_args;

    if let Some(addr) = blend_addr {
        blend.backend.listening_address = addr;
    }

    if let Some(node_key) = blend_node_key {
        let mut key_bytes = hex::decode(node_key)?;
        blend.backend.node_key = SecretKey::try_from_bytes(key_bytes.as_mut_slice())?;
    }

    if let Some(num_blend_layers) = blend_num_blend_layers {
        blend.message_blend.cryptographic_processor.num_blend_layers = num_blend_layers;
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
        note_secret_key,
        note_value,
        note_nonce,
    } = consensus_args;

    if let (Some(value), Some(nonce)) = (note_value, note_nonce) {
        let nonce = Nonce::from_bytes(<[u8; 32]>::from_hex(nonce)?);
        cryptarchia.leader_config.notes.push(NoteWitness::new(
            value.into(),
            NMO_UNIT,
            CovenantProof::nop_constraint(),
            [0; 32],
            nonce,
        ));
    }

    if let Some(sk) = note_secret_key {
        let sk = <[u8; 16]>::from_hex(sk)?;
        cryptarchia.leader_config.nf_sk = NullifierSecret::from_bytes(sk);
    }

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
