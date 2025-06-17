use std::collections::HashSet;

use cryptarchia_sync::{ChainSyncError, HeaderId, SerialisedBlock, SerialisedHeaderId};
use futures::stream::BoxStream;
use libp2p::PeerId;
use tokio::sync::{mpsc::Sender, oneshot};

use crate::{behaviour::BehaviourError, Swarm};

impl Swarm {
    pub fn request_tip(
        &self,
        peer_id: PeerId,
        reply_sender: oneshot::Sender<Result<SerialisedHeaderId, ChainSyncError>>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour().chain_sync.as_ref() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .request_tip(peer_id, reply_sender)
            .map_err(Into::into)
    }

    pub fn start_blocks_download(
        &self,
        peer_id: PeerId,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: HashSet<HeaderId>,
        reply_sender: Sender<BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>>,
    ) -> Result<(), BehaviourError> {
        let Some(chain_sync) = self.swarm.behaviour().chain_sync.as_ref() else {
            return Err(BehaviourError::OperationNotSupported);
        };

        chain_sync
            .start_blocks_download(
                peer_id,
                target_block,
                local_tip,
                latest_immutable_block,
                additional_blocks,
                reply_sender,
            )
            .map_err(Into::into)
    }
}
