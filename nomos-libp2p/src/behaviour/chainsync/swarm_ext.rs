use cryptarchia_sync::{BlocksResponse, HeaderId};
use tokio::sync::mpsc;

use crate::{behaviour::BehaviourError, Swarm};

impl Swarm {
    pub fn start_blocks_download(
        &mut self,
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        reply_rcv: mpsc::Sender<BlocksResponse>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour_mut().chain_sync.as_mut() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .start_blocks_download(
                target_block,
                local_tip,
                immutable_block,
                additional_blocks,
                reply_rcv,
            )
            .map_err(Into::into)
    }
}
