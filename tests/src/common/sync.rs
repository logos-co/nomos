use std::time::Duration;

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

        println!(
            "   Initial validators: [{}]",
            infos
                .iter()
                .map(|info| format!("{:?}/{:?}", info.height, info.mode))
                .collect::<Vec<_>>()
                .join(", ")
        );

        if infos.iter().all(|info| info.mode == mode)
            && infos.iter().all(|info| info.height >= min_height)
        {
            println!("   All validators reached are in mode {mode:?}",);
            break;
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}
