use std::time::Duration;

use chain_service::CryptarchiaInfo;
use futures_util::{stream, StreamExt as _};
use tokio::time::timeout;

use crate::nodes::validator::Validator;

pub async fn wait_for_validators_mode_and_height(
    validators: &[Validator],
    mode: cryptarchia_engine::State,
    min_height: u64,
    timeout_duration: Duration,
) {
    timeout(timeout_duration, async {
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
                return;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Timeout ({timeout_duration:?}) waiting for validators to reach mode {mode:?} and height {min_height}",
        )
    });
}

pub async fn wait_for_validators_mode(
    validators: &[&Validator],
    mode: cryptarchia_engine::State,
    timeout_duration: Duration,
) {
    timeout(timeout_duration, async {
        loop {
            let infos: Vec<_> = stream::iter(validators)
                .then(|n| async move { n.consensus_info().await })
                .collect()
                .await;
            print_validators_info(&infos);

            if infos.iter().all(|info| info.mode == mode) {
                println!("   All validators reached are in mode {mode:?}");
                return;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("Timeout ({timeout_duration:?}) waiting for validators to reach mode {mode:?}",)
    });
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
