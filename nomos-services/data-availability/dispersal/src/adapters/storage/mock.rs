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

    fn last_tx(&self, channel_id: &ChannelId) -> Option<SignedMantleTx> {
        self.transactions.get(channel_id).cloned()
    }

    fn last_tx_id(&self, channel_id: &ChannelId) -> MsgId {
        self.transactions.get(channel_id).map_or_else(
            || {
                [
                    170, 5, 74, 138, 86, 29, 83, 52, 191, 137, 209, 156, 189, 202, 136, 2, 253,
                    158, 106, 195, 32, 145, 82, 22, 3, 187, 58, 93, 109, 99, 235, 164,
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
