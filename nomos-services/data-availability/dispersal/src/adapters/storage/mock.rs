use std::collections::HashMap;

use nomos_core::mantle::{
    AuthenticatedMantleTx as _, Op, SignedMantleTx,
    ops::channel::{ChannelId, MsgId},
};

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

    fn last_tx_id(&self, channel_id: &ChannelId) -> MsgId {
        self.transactions.get(channel_id).map_or_else(
            || {
                // Inscription op id used in integration tests
                [
                    231, 225, 221, 65, 226, 6, 22, 89, 92, 231, 197, 121, 113, 142, 78, 120, 84,
                    73, 87, 89, 153, 26, 63, 24, 87, 217, 30, 151, 200, 135, 40, 235,
                ]
                .into()
            },
            |last_tx| {
                let Some((Op::ChannelBlob(blob_op), _)) = last_tx.ops_with_proof().next() else {
                    panic!("Previously sent transaction should have a blob operation");
                };
                blob_op.id()
            },
        )
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
