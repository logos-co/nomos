use futures::{
    future::{join_all, BoxFuture},
    AsyncWriteExt, FutureExt,
};
use libp2p::PeerId;
use libp2p_stream::Control;
use tokio::{
    sync::mpsc::UnboundedSender,
    time::{timeout, Duration},
};
use tracing::{error, info};

use crate::{
    behaviour::{SyncDirection, SyncError, SyncRequest, SYNC_PROTOCOL},
    membership::ConsensusMembershipHandler,
    sync_utils,
};

pub async fn select_best_peer_for_sync<Membership>(
    control: Control,
    membership: Membership,
    local_peer_id: PeerId,
) -> Result<PeerId, SyncError>
where
    Membership: ConsensusMembershipHandler<Id = PeerId> + Clone + 'static + Send,
{
    let peers: Vec<PeerId> = membership
        .members()
        .iter()
        .filter(|&&id| id != local_peer_id)
        .copied()
        .collect();

    if peers.is_empty() {
        return Err(SyncError::NoPeersAvailable);
    }

    info!(num_peers = peers.len(), "Selecting best peer for sync");

    let tip_futures: Vec<_> = peers
        .iter()
        .map(|&peer_id| {
            // Probably should do something about peers that timeout
            timeout(
                Duration::from_secs(5),
                request_tip_from_peer(peer_id, control.clone()),
            )
        })
        .collect();

    let tips_results = join_all(tip_futures).await;
    let mut best_peer = None;
    let mut highest_tip = 0;

    for (peer_id, result) in peers.iter().zip(tips_results) {
        match result {
            Ok(Ok(tip)) => {
                info!(peer_id = %peer_id, tip = tip, "Received tip");
                if tip > highest_tip {
                    highest_tip = tip;
                    best_peer = Some(*peer_id);
                }
            }
            Ok(Err(e)) => error!(peer_id = %peer_id, error = %e, "Failed to get tip"),
            Err(_) => error!(peer_id = %peer_id, "Timeout getting tip"),
        }
    }

    best_peer.ok_or(SyncError::NoPeersAvailable)
}

pub async fn stream_blocks_from_peer(
    peer_id: PeerId,
    control: &mut Control,
    direction: SyncDirection,
    response_sender: UnboundedSender<(Vec<u8>, PeerId)>,
) -> Result<(), SyncError> {
    info!(peer_id = %peer_id, "Streaming blocks");

    let mut stream = sync_utils::open_stream(peer_id, control, SYNC_PROTOCOL).await?;
    sync_utils::send_data(&mut stream, &SyncRequest::Sync { direction }).await?;

    while let Ok(block) = sync_utils::receive_data(&mut stream).await {
        if response_sender.send((block, peer_id)).is_err() {
            break;
        }
    }
    stream.close().await?;

    info!(peer_id = %peer_id, "Finished streaming blocks");
    Ok(())
}

async fn request_tip_from_peer(peer_id: PeerId, mut control: Control) -> Result<u64, SyncError> {
    let mut stream = sync_utils::open_stream(peer_id, &mut control, SYNC_PROTOCOL).await?;
    sync_utils::send_data(&mut stream, &SyncRequest::RequestTip).await?;

    let tip = sync_utils::receive_data(&mut stream).await?;
    stream.close().await?;

    info!(peer_id = %peer_id, tip = tip, "Received tip");
    Ok(tip)
}

pub fn sync_after_requesting_tips<Membership>(
    control: Control,
    membership: Membership,
    local_peer_id: PeerId,
    direction: SyncDirection,
    response_sender: UnboundedSender<(Vec<u8>, PeerId)>,
) -> BoxFuture<'static, Result<(), SyncError>>
where
    Membership: ConsensusMembershipHandler<Id = PeerId> + Clone + 'static + Send,
{
    async move {
        match direction {
            SyncDirection::Forward(_start_slot) => {
                // TODO: use start_slot to filter out peers that aren't ahead of us.
                let best_peer =
                    select_best_peer_for_sync(control.clone(), membership, local_peer_id).await?;

                info!(peer_id = %best_peer, "Chosen peer for sync");
                stream_blocks_from_peer(best_peer, &mut control.clone(), direction, response_sender)
                    .await
            }
            SyncDirection::Backward { peer, .. } => {
                // NOTE: Actually, the `peer` info included in the `direction` doesn't need to
                //       be sent to the peer via network. But just not refactor this yet.
                stream_blocks_from_peer(peer, &mut control.clone(), direction, response_sender)
                    .await
            }
        }
    }
    .boxed()
}
