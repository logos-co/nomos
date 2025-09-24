use clap::Parser as _;
use color_eyre::eyre::{Result, eyre};
use nomos_node::{Config, config::CliArgs, get_services_to_start, run_node_from_config};

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

    let app = run_node_from_config(config).map_err(|e| eyre!("{e}"))?;

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
