use clap::Parser;
use color_eyre::eyre::{Result, eyre};
use nomos_core::mantle::SignedMantleTx;
use nomos_executor::{
    NomosExecutor, NomosExecutorServiceSettings, RuntimeServiceId,
    config::{Config as ExecutorConfig, da::ServiceConfig as DaConfig},
};
use nomos_node::{
    CryptarchiaLeaderArgs, HttpArgs, LogArgs, MANTLE_TOPIC, MempoolAdapterSettings, NetworkArgs,
    Transaction,
    config::{
        BlendArgs, blend::ServiceConfig as BlendConfig, network::ServiceConfig as NetworkConfig,
    },
};
use nomos_sdp::SdpSettings;
use overwatch::overwatch::{Error as OverwatchError, Overwatch, OverwatchRunner};
use tx_service::tx::settings::TxMempoolSettings;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path for a yaml-encoded network config file
    config: std::path::PathBuf,
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
    cryptarchia_leader: CryptarchiaLeaderArgs,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Args {
        config,
        log: log_args,
        http: http_args,
        network: network_args,
        blend: blend_args,
        cryptarchia_leader: cryptarchia_args,
        check_config_only,
    } = Args::parse();
    let config = serde_yaml::from_reader::<_, ExecutorConfig>(std::fs::File::open(config)?)?
        .update_from_args(
            log_args,
            network_args,
            blend_args,
            http_args,
            cryptarchia_args,
        )?;

    #[expect(
        clippy::non_ascii_literal,
        reason = "Use of green checkmark for better UX."
    )]
    if check_config_only {
        println!("Config file is valid! ✅");
        return Ok(());
    }

    let node_deployment: nomos_node::config::deployment::Settings =
        config.deployment.clone().into();

    let (blend_config, blend_core_config, blend_edge_config) = BlendConfig {
        user: config.blend,
        deployment: node_deployment.clone().into(),
    }
    .into();

    let (da_network_config, da_verifier_config, da_sampling_config, da_dispersal_config) =
        DaConfig {
            user: config.da,
            deployment: config.deployment.clone().into(),
        }
        .into();

    let app = OverwatchRunner::<NomosExecutor>::run(
        NomosExecutorServiceSettings {
            network: NetworkConfig {
                user: config.network,
                deployment: node_deployment.clone().into(),
            }
            .into(),
            blend: blend_config,
            blend_core: blend_core_config,
            blend_edge: blend_edge_config,
            block_broadcast: (),
            #[cfg(feature = "tracing")]
            tracing: config.tracing,
            http: config.http,
            mempool: TxMempoolSettings {
                pool: (),
                network_adapter: MempoolAdapterSettings {
                    topic: String::from(MANTLE_TOPIC),
                    id: <SignedMantleTx as Transaction>::hash,
                },
                recovery_path: config.mempool.pool_recovery_path,
            },
            da_dispersal: da_dispersal_config,
            da_network: da_network_config,
            da_sampling: da_sampling_config,
            da_verifier: da_verifier_config,
            cryptarchia: config.cryptarchia,
            chain_network: config.chain_network,
            cryptarchia_leader: config.cryptarchia_leader,
            time: config.time,
            storage: config.storage,
            system_sig: (),
            sdp: SdpSettings { declaration: None },
            wallet: config.wallet,
            key_management: config.key_management,
            #[cfg(feature = "testing")]
            testing_http: config.testing_http,
        },
        None,
    )
    .map_err(|e| eyre!("Error encountered: {}", e))?;

    drop(
        app.handle()
            .start_service_sequence(get_services_to_start(&app).await?)
            .await,
    );
    app.wait_finished().await;
    Ok(())
}

async fn get_services_to_start(
    app: &Overwatch<RuntimeServiceId>,
) -> Result<Vec<RuntimeServiceId>, OverwatchError> {
    let mut service_ids = app.handle().retrieve_service_ids().await?;

    // Exclude core and edge blend services, which will be started
    // on demand by the blend service.
    let blend_inner_service_ids = [RuntimeServiceId::BlendCore, RuntimeServiceId::BlendEdge];
    service_ids.retain(|value| !blend_inner_service_ids.contains(value));

    Ok(service_ids)
}
