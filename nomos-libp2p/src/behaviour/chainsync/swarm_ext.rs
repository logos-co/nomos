use cryptarchia_sync::{Block, HeaderId};
use tokio::sync::mpsc;

use crate::{behaviour::BehaviourError, Swarm};

impl Swarm {
    pub fn start_chain_sync(
        &mut self,
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
        rx_blocks: mpsc::Sender<Block>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour_mut().chain_sync.as_mut() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .start_sync(
                target_block,
                local_tip,
                immutable_block,
                additional_blocks,
                rx_blocks,
            )
            .map_err(Into::into)
    }
}
