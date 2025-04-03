use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use kzgrs_backend::dispersal::BlobInfo;
use nomos_core::{da::blob::info::DispersedBlobInfo, tx::Transaction};
use nomos_mempool::{
    network::adapters::libp2p::Settings as AdapterSettings, tx::settings::TxMempoolSettings,
};
use nomos_node::{config::CliArgs, Config, Nomos, NomosServiceSettings, Tx};
use overwatch::overwatch::OverwatchRunner;

fn main() -> Result<()> {
    let cli_args = CliArgs::parse();
    let config =
        serde_yaml::from_reader::<_, Config>(std::fs::File::open(cli_args.config_path())?)?
            .update_from_args(cli_args)?;

    let app = OverwatchRunner::<Nomos>::run(
        NomosServiceSettings {
            network: config.network,
            blend: config.blend,
            #[cfg(feature = "tracing")]
            tracing: config.tracing,
            http: config.http,
            cl_mempool: TxMempoolSettings {
                pool: (),
                network_adapter: AdapterSettings {
                    topic: String::from(nomos_node::CL_TOPIC),
                    id: <Tx as Transaction>::hash,
                },
                recovery_path: config.mempool.cl_pool_recovery_path,
            },
            da_mempool: nomos_mempool::DaMempoolSettings {
                pool: (),
                network_adapter: AdapterSettings {
                    topic: String::from(nomos_node::DA_TOPIC),
                    id: <BlobInfo as DispersedBlobInfo>::blob_id,
                },
                recovery_path: config.mempool.da_pool_recovery_path,
            },
            da_network: config.da_network,
            da_indexer: config.da_indexer,
            da_sampling: config.da_sampling,
            da_verifier: config.da_verifier,
            cryptarchia: config.cryptarchia,
            time: config.time,
            storage: config.storage,
            system_sig: (),
        },
        None,
    )
    .map_err(|e| eyre!("Error encountered: {}", e))?;
    app.wait_finished();
    Ok(())
}
