use std::collections::HashMap;

use nomos_core::mantle::{ops::channel::ChannelId, SignedMantleTx};

use super::DispersalStorageAdapter;

pub struct MockDispersalStorageAdapter {
    transactions: HashMap<ChannelId, SignedMantleTx>,
}

impl DispersalStorageAdapter for MockDispersalStorageAdapter {
    fn new() -> Self {
        Self {
            transactions: HashMap::new(),
        }
    }

    fn last_tx(&self, channel_id: &ChannelId) -> Option<SignedMantleTx> {
        self.transactions.get(channel_id).cloned()
    }

    fn store_tx(
        &mut self,
        channel_id: ChannelId,
        tx: SignedMantleTx,
    ) -> Result<(), super::DispersalStorageError> {
        self.transactions.insert(channel_id, tx);
        Ok(())
    }
}
