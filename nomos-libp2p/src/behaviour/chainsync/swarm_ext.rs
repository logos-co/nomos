use cryptarchia_sync::{DownloadBlocksInfo, HeaderId};

use crate::{behaviour::BehaviourError, Swarm};

impl Swarm {
    pub fn start_blocks_download(
        &mut self,
        peer_id: libp2p::PeerId,
        target_block: Option<HeaderId>,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: Vec<HeaderId>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour_mut().chain_sync.as_mut() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .start_blocks_download(
                peer_id,
                DownloadBlocksInfo {
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks,
                },
            )
            .map_err(Into::into)
    }
}
