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

    let (blend_config, blend_core_config, blend_edge_config) = config.blend.into();

    let app = OverwatchRunner::<Nomos>::run(
        NomosServiceSettings {
            network: config.network,
            blend: blend_config,
            blend_core: blend_core_config,
            blend_edge: blend_edge_config,
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
                trigger_sampling_delay: config.mempool.trigger_sampling_delay,
            },
            da_network: config.da_network,
            da_sampling: config.da_sampling,
            da_verifier: config.da_verifier,
            cryptarchia: config.cryptarchia,
            time: config.time,
            storage: config.storage,
            system_sig: (),
            sdp: (),
            membership: config.membership,
            wallet: wallet::WalletServiceSettings {
                known_keys: {
                    let mut keys = std::collections::HashSet::new();
                    let sample_key = nomos_core::mantle::keys::PublicKey::from(
                        num_bigint::BigUint::from(12345u64),
                    );
                    keys.insert(sample_key);
                    keys
                },
            },
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

    // Exclude core and edge blend services, which will be started
    // on demand by the blend service.
    let blend_inner_service_ids = [RuntimeServiceId::BlendCore, RuntimeServiceId::BlendEdge];
    service_ids.retain(|value| !blend_inner_service_ids.contains(value));

    if !must_blend_service_group_start {
        service_ids.retain(|value| value != &RuntimeServiceId::Blend);
    }

    if !must_da_service_group_start {
        let da_service_ids = [
            RuntimeServiceId::DaVerifier,
            RuntimeServiceId::DaSampling,
            RuntimeServiceId::DaNetwork,
            RuntimeServiceId::DaMempool,
        ];
        service_ids.retain(|value| !da_service_ids.contains(value));
    }

    Ok(service_ids)
}

#[cfg(test)]
mod tests {
    use nomos_core::mantle::{keys::PublicKey, Note, TxHash, Utxo};
    use num_bigint::BigUint;

    #[test]
    fn test_blah() {
        let utxo = Utxo {
            tx_hash: TxHash::from(BigUint::from(0u64)),
            output_index: 0,
            note: Note {
                value: 1,
                pk: PublicKey::from(BigUint::from(0u64)),
            },
        };

        println!("{}", &serde_yaml::to_string(&(utxo.id(), utxo)).unwrap());
        panic!()
    }
}
