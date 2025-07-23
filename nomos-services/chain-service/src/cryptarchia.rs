use std::pin::Pin;

use cryptarchia_engine::{Boostrapping, Online, PrunedBlocks};
use nomos_core::header::HeaderId;
use tokio::time::{Instant, Sleep};

use crate::{BootstrapConfig, Cryptarchia};

pub trait CryptarchiaExt {
    fn prolonged_boostrap_timer(&self, config: &BootstrapConfig) -> Option<Pin<Box<Sleep>>>;
    fn online(self) -> (Cryptarchia<Online>, PrunedBlocks<HeaderId>);
}

impl CryptarchiaExt for Cryptarchia<Boostrapping> {
    fn prolonged_boostrap_timer(&self, config: &BootstrapConfig) -> Option<Pin<Box<Sleep>>> {
        Some(Box::pin(tokio::time::sleep_until(
            Instant::now() + config.prolonged_bootstrap_period,
        )))
    }

    fn online(self) -> (Cryptarchia<Online>, PrunedBlocks<HeaderId>) {
        let (consensus, pruned_blocks) = self.consensus.online();
        (
            Cryptarchia {
                ledger: self.ledger,
                consensus,
            },
            pruned_blocks,
        )
    }
}

impl CryptarchiaExt for Cryptarchia<Online> {
    fn prolonged_boostrap_timer(&self, _config: &BootstrapConfig) -> Option<Pin<Box<Sleep>>> {
        None
    }

    fn online(self) -> (Cryptarchia<Online>, PrunedBlocks<HeaderId>) {
        (self, PrunedBlocks::default())
    }
}
