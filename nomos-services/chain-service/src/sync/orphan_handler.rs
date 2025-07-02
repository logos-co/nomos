use crate::network::NetworkAdapter;
use futures::StreamExt;
use nomos_core::block::Block;
use nomos_core::header::HeaderId;
use overwatch::DynError;
use std::collections::HashSet;
use tracing::info;

pub struct OrphanHandler<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId> + Send + Sync,
{
    _phantom: std::marker::PhantomData<(NetAdapter, RuntimeServiceId)>,
}

impl<NetAdapter, Tx, BlockCert, RuntimeServiceId> OrphanHandler<NetAdapter, RuntimeServiceId>
where
    NetAdapter: NetworkAdapter<RuntimeServiceId, Block = Block<Tx, BlockCert>> + Send + Sync,
    Tx: Clone + Eq + Send + Sync + 'static,
    BlockCert: Clone + Eq + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    pub async fn request_parent_block(
        &mut self,
        network_adapter: &NetAdapter,
        parent_id: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
    ) -> Result<Vec<Block<Tx, BlockCert>>, DynError> {
        // Track this to avoid downloading the same block multiple times.
        let mut latest_downloaded_block: Option<HeaderId> = None;

        let mut blocks = vec![];

        loop {
            let mut blocks_stream = network_adapter
                .request_blocks_from_peers(
                    parent_id,
                    local_tip,
                    latest_immutable_block,
                    latest_downloaded_block
                        .map(|id| HashSet::from([id]))
                        .unwrap_or_default(),
                )
                .await?;

            info!("ORPHAN: Receiving blocks from peers...");

            loop {
                let block_result = blocks_stream.next().await;
                match block_result {
                    Some(Ok(block)) => {
                        let block_id = block.header().id();
                        info!("ORPHAN: Received block: {block_id}");

                        latest_downloaded_block = Some(block_id);

                        blocks.push(block);
                    }
                    Some(Err(e)) => {
                        info!("ORPHAN: Error receiving block: {e}");
                        return Err(e);
                    }
                    None => {
                        info!("ORPHAN: No more blocks in the stream.");
                        return Ok(blocks);
                    },
                }
            }
        }
    }
}
