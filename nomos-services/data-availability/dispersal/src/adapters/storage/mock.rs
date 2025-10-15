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
                    56, 175, 135, 67, 106, 190, 160, 137, 218, 122, 2, 114, 80, 39, 178, 39, 52,
                    241, 73, 61, 40, 175, 174, 105, 112, 210, 30, 40, 139, 8, 90, 162,
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
