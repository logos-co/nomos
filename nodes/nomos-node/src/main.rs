use full_replication::Certificate;
#[cfg(feature = "metrics")]
use nomos_metrics::MetricsSettings;
use nomos_node::{
    Config, CryptarchiaArgs, HttpArgs, LogArgs, MetricsArgs, NetworkArgs, Nomos,
    NomosServiceSettings, Tx,
};

use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use nomos_core::{da::certificate, tx::Transaction};

use nomos_mempool::network::adapters::libp2p::Settings as AdapterSettings;

use overwatch_rs::overwatch::*;

const DEFAULT_DB_PATH: &str = "./db";

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path for a yaml-encoded network config file
    config: std::path::PathBuf,
    /// Overrides log config.
    #[clap(flatten)]
    log_args: LogArgs,
    /// Overrides network config.
    #[clap(flatten)]
    network_args: NetworkArgs,
    /// Overrides http config.
    #[clap(flatten)]
    http_args: HttpArgs,
    #[clap(flatten)]
    cryptarchia_args: CryptarchiaArgs,
    /// Overrides metrics config.
    #[clap(flatten)]
    metrics_args: MetricsArgs,
}

fn main() -> Result<()> {
    let Args {
        config,
        log_args,
        http_args,
        network_args,
        cryptarchia_args,
        metrics_args,
    } = Args::parse();
    let config = serde_yaml::from_reader::<_, Config>(std::fs::File::open(config)?)?
        .update_log(log_args)?
        .update_http(http_args)?
        .update_network(network_args)?
        .update_cryptarchia_consensus(cryptarchia_args)?;

    let registry = cfg!(feature = "metrics")
        .then(|| {
            metrics_args
                .with_metrics
                .then(nomos_metrics::NomosRegistry::default)
        })
        .flatten();

    let app = OverwatchRunner::<Nomos>::run(
        NomosServiceSettings {
            network: config.network,
            #[cfg(feature = "tracing")]
            logging: config.log,
            http: config.http,
            cl_mempool: nomos_mempool::TxMempoolSettings {
                backend: (),
                network: AdapterSettings {
                    topic: String::from(nomos_node::CL_TOPIC),
                    id: <Tx as Transaction>::hash,
                },
                registry: registry.clone(),
            },
            da_mempool: nomos_mempool::DaMempoolSettings {
                backend: (),
                network: AdapterSettings {
                    topic: String::from(nomos_node::DA_TOPIC),
                    id: <Certificate as certificate::Certificate>::id,
                },
                verification_provider: full_replication::CertificateVerificationParameters {
                    threshold: 0,
                },
                registry: registry.clone(),
            },
            cryptarchia: config.cryptarchia,
            #[cfg(feature = "metrics")]
            metrics: MetricsSettings { registry },
            storage: nomos_storage::backends::rocksdb::RocksBackendSettings {
                db_path: std::path::PathBuf::from(DEFAULT_DB_PATH),
                read_only: false,
                column_family: Some("blocks".into()),
            },
            system_sig: (),
        },
        None,
    )
    .map_err(|e| eyre!("Error encountered: {}", e))?;
    app.wait_finished();
    Ok(())
}
