use std::time::Duration;

use cryptarchia_engine::Boostrapping;
use tokio::time::interval;
use tokio_stream::{adapters::Take, wrappers::IntervalStream, StreamExt};

use crate::{wrapper::CryptarchiaWrapper, Cryptarchia};

pub fn select_fork_choice_rule(cryptarchia: Cryptarchia<Boostrapping>) -> CryptarchiaWrapper {
    // Choose Bootstrapping if the node is starting from the genesis block.
    // TODO: Also choose Bootstrapping if the node is starting from a checkpoint.
    // https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df8136b6ffd9a910da0a6b
    if cryptarchia.consensus.lib_branch().is_genesis() {
        return CryptarchiaWrapper::Bootstrapping(cryptarchia);
    }

    // TODO: Choose Bootstrapping if a node is restarting
    // after being offline longer than "Offline Grace Period".
    // https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df81cfaffef835bc6f0e68

    // TODO: Choose Bootstrapping if `--bootstrap` flag is set.
    // https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df81e38821d6911f78dba3

    CryptarchiaWrapper::Online(cryptarchia.online())
}

pub fn new_prolonged_bootstrap_timer(
    cryptarchia: &CryptarchiaWrapper,
    period: Duration,
) -> Take<IntervalStream> {
    let period = match cryptarchia {
        CryptarchiaWrapper::Bootstrapping(_) => period,
        CryptarchiaWrapper::Online(_) => Duration::ZERO,
    };
    IntervalStream::new(interval(period)).take(1)
}
