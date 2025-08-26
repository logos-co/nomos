use std::time::Duration;

use chain_service::CryptarchiaInfo;
use futures_util::{stream, StreamExt as _};

use crate::nodes::validator::Validator;

pub async fn wait_for_validators_mode_and_height(
    validators: &[Validator],
    mode: cryptarchia_engine::State,
    min_height: u64,
) {
    loop {
        let infos: Vec<_> = stream::iter(validators)
            .then(|n| async move { n.consensus_info().await })
            .collect()
            .await;
        print_validators_info(&infos);

        if infos.iter().all(|info| info.mode == mode)
            && infos.iter().all(|info| info.height >= min_height)
        {
            println!("   All validators reached are in mode {mode:?} and height {min_height}");
            break;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub async fn wait_for_validators_mode(validators: &[&Validator], mode: cryptarchia_engine::State) {
    loop {
        let infos: Vec<_> = stream::iter(validators)
            .then(|n| async move { n.consensus_info().await })
            .collect()
            .await;
        print_validators_info(&infos);

        if infos.iter().all(|info| info.mode == mode) {
            println!("   All validators reached are in mode {mode:?}");
            break;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn print_validators_info(infos: &[CryptarchiaInfo]) {
    println!(
        "   Validators: {:?}",
        infos
            .iter()
            .map(|info| format!("{:?}/{:?}", info.height, info.mode))
            .collect::<Vec<_>>(),
    );
}
