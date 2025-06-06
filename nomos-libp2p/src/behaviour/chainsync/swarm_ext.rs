use cryptarchia_sync::HeaderId;

use crate::{behaviour::BehaviourError, Swarm};

impl Swarm {
    pub fn start_blocks_download(
        &mut self,
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour_mut().chain_sync.as_mut() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .start_blocks_download(target_block, local_tip, immutable_block, additional_blocks)
            .map_err(Into::into)
    }
}
