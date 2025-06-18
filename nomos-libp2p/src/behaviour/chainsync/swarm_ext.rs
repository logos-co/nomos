use std::collections::HashSet;

use cryptarchia_sync::{ChainSyncError, HeaderId, SerialisedBlock};
use futures::stream::BoxStream;
use libp2p::PeerId;
use tokio::sync::{mpsc::Sender, oneshot};

use crate::{behaviour::BehaviourError, Swarm};

type SerialisedBlockStream = BoxStream<'static, Result<SerialisedBlock, ChainSyncError>>;

impl Swarm {
    pub fn request_tip(
        &self,
        peer_id: PeerId,
        reply_sender: oneshot::Sender<Result<HeaderId, ChainSyncError>>,
    ) -> Result<(), BehaviourError> {
        let chain_sync = &self.swarm.behaviour().chain_sync;

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
        reply_sender: Sender<SerialisedBlockStream>,
    ) -> Result<(), BehaviourError> {
        let chain_sync = &self.swarm.behaviour().chain_sync;

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
