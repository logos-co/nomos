use clap::Parser as _;
use color_eyre::eyre::{eyre, Result};
use kzgrs_backend::dispersal::BlobInfo;
use nomos_core::{
    da::blob::info::DispersedBlobInfo,
    mantle::{SignedMantleTx, Transaction},
};
use nomos_mempool::{
    network::adapters::libp2p::Settings as AdapterSettings, tx::settings::TxMempoolSettings,
};
use nomos_node::{config::CliArgs, Config, Nomos, NomosServiceSettings, RuntimeServiceId};
use overwatch::overwatch::{Error as OverwatchError, Overwatch, OverwatchRunner};

#[tokio::main]
async fn main() -> Result<()> {
    let cli_args = CliArgs::parse();
    let is_dry_run = cli_args.dry_run();
    let must_blend_service_group_start = cli_args.must_blend_service_group_start();
    let must_da_service_group_start = cli_args.must_da_service_group_start();

    let config =
        serde_yaml::from_reader::<_, Config>(std::fs::File::open(cli_args.config_path())?)?
            .update_from_args(cli_args)?;

    #[expect(
        clippy::non_ascii_literal,
        reason = "Use of green checkmark for better UX."
    )]
    if is_dry_run {
        println!("Config file is valid! ✅");
        return Ok(());
    }

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
                    id: <SignedMantleTx as Transaction>::hash,
                },
                processor: (),
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
            sdp: (),
            membership: config.membership,
            #[cfg(feature = "testing")]
            testing_http: config.testing_http,
        },
        None,
    )
    .map_err(|e| eyre!("Error encountered: {}", e))?;

    let services_to_start = get_services_to_start(
        &app,
        must_blend_service_group_start,
        must_da_service_group_start,
    )
    .await?;

    let _ = app.handle().start_service_sequence(services_to_start).await;

    app.wait_finished().await;
    Ok(())
}

async fn get_services_to_start(
    app: &Overwatch<RuntimeServiceId>,
    must_blend_service_group_start: bool,
    must_da_service_group_start: bool,
) -> Result<Vec<RuntimeServiceId>, OverwatchError> {
    let mut service_ids = app.handle().retrieve_service_ids().await?;

    if !must_blend_service_group_start {
        service_ids.retain(|value| value != &RuntimeServiceId::Blend);
    }

    if !must_da_service_group_start {
        let da_service_ids = [
            RuntimeServiceId::DaIndexer,
            RuntimeServiceId::DaVerifier,
            RuntimeServiceId::DaSampling,
            RuntimeServiceId::DaNetwork,
            RuntimeServiceId::DaMempool,
        ];
        service_ids.retain(|value| !da_service_ids.contains(value));
    }

    Ok(service_ids)
}
