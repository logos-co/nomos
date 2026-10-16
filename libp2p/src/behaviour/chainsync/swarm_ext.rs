use std::collections::HashSet;

use lb_cryptarchia_sync::{
    BoxedStream, ChainSyncError, ChainSyncErrorKind, GetTipResponse, GetTipResponseReason,
    HeaderId, SerialisedBlock,
};
use libp2p::PeerId;
use rand::RngCore;
use tokio::sync::oneshot;

use crate::{Swarm, behaviour::BehaviourError};

type SerialisedBlockStream = BoxedStream<Result<SerialisedBlock, ChainSyncError>>;

impl<R: Clone + Send + RngCore + 'static> Swarm<R> {
    fn chain_sync_is_blocked(&self, peer_id: PeerId) -> bool {
        self.global_peer_block_predicate
            .as_ref()
            .is_some_and(|predicate| predicate(peer_id))
            || self
                .chain_sync_peer_block_predicate
                .as_ref()
                .is_some_and(|predicate| predicate(peer_id))
    }

    pub fn request_tip(
        &self,
        peer_id: PeerId,
        reply_sender: oneshot::Sender<Result<GetTipResponse, ChainSyncError>>,
    ) -> Result<(), BehaviourError> {
        if self.chain_sync_is_blocked(peer_id) {
            let error = ChainSyncError::new(
                peer_id,
                ChainSyncErrorKind::RequestTipError(GetTipResponseReason::NodeNotOnline),
            );
            drop(reply_sender.send(Err(error.clone())));
            return Err(BehaviourError::ChainSyncError(error));
        }

        let chain_sync = &self.swarm.behaviour().inner.chain_sync;

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
        reply_sender: oneshot::Sender<SerialisedBlockStream>,
    ) -> Result<(), BehaviourError> {
        if self.chain_sync_is_blocked(peer_id) {
            let error = ChainSyncError::new(
                peer_id,
                ChainSyncErrorKind::RequestBlocksDownloadError(
                    "peer is banned for ChainSync or globally".to_owned(),
                ),
            );
            let stream_error = error.clone();
            let stream: SerialisedBlockStream =
                Box::new(futures::stream::iter([Err(stream_error)]));
            drop(reply_sender.send(stream));
            return Err(BehaviourError::ChainSyncError(error));
        }

        let chain_sync = &self.swarm.behaviour().inner.chain_sync;

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
