use std::sync::Arc;

use nomos_core::{
    block::Block,
    mantle::{
        AuthenticatedMantleTx as _, SignedMantleTx,
        ops::{Op, channel::MsgId},
    },
};
use testing_framework_core::scenario::{DynError, RunContext};

/// Scans a block and invokes the matcher for every operation until it returns
/// `Some(...)`. Returns `None` when no matching operation is found.
pub fn find_channel_op<F>(block: &Block<SignedMantleTx>, matcher: &mut F) -> Option<MsgId>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    for tx in block.transactions() {
        for op in &tx.mantle_tx().ops {
            if let Some(msg_id) = matcher(op) {
                return Some(msg_id);
            }
        }
    }

    None
}

pub async fn submit_transaction_via_cluster(
    ctx: &RunContext,
    tx: Arc<SignedMantleTx>,
) -> Result<(), DynError> {
    ctx.cluster_client()
        .try_all_clients(|client| {
            let tx = Arc::clone(&tx);
            Box::pin(async move {
                client
                    .submit_transaction(&tx)
                    .await
                    .map_err(|err| -> DynError { err.into() })
            })
        })
        .await
}
