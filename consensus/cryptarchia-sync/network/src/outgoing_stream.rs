use std::future::Future;

use cryptarchia_engine::Slot;
use futures::{AsyncWriteExt, FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use libp2p::PeerId;
use libp2p_stream::Control;
use tokio::{
    sync::mpsc::Sender,
    time::{Duration, error::Elapsed, timeout},
};
use tracing::{error, info};

use crate::{
    behaviour::{SYNC_PROTOCOL, SyncError},
    membership::ConnectedPeers,
    messages::{SyncDirection, SyncRequest, SyncResponse},
    sync_utils,
};

type TipResult = Result<Result<(PeerId, Slot), SyncError>, Elapsed>;

/// Find the peer with the highest tip among the given peers.
pub async fn select_best_peer_for_sync(
    control: Control,
    peers: Vec<PeerId>,
    local_peer_id: PeerId,
) -> Result<PeerId, SyncError> {
    let filtered_peers = filter_valid_peers(peers, local_peer_id)?;
    let tip_requests = create_tip_requests(filtered_peers, &control);
    let best_peer = find_highest_tip_peer(tip_requests).await?;
    Ok(best_peer)
}

fn filter_valid_peers(peers: Vec<PeerId>, local_peer_id: PeerId) -> Result<Vec<PeerId>, SyncError> {
    let filtered: Vec<PeerId> = peers
        .into_iter()
        .filter(|&id| id != local_peer_id)
        .collect();

    if filtered.is_empty() {
        return Err(SyncError::NoPeersAvailable);
    }

    info!(num_peers = filtered.len(), "Selecting best peer for sync");
    Ok(filtered)
}

fn create_tip_requests(
    peers: Vec<PeerId>,
    control: &Control,
) -> FuturesUnordered<impl Future<Output = TipResult>> {
    peers
        .into_iter()
        .map(|peer_id| {
            let control = control.clone();
            async move {
                timeout(
                    Duration::from_secs(5),
                    request_tip_from_peer(peer_id, control),
                )
                .await
            }
        })
        .collect::<FuturesUnordered<_>>()
}

async fn find_highest_tip_peer(
    mut tip_requests: FuturesUnordered<
        impl Future<Output = Result<Result<(PeerId, Slot), SyncError>, Elapsed>>,
    >,
) -> Result<PeerId, SyncError> {
    let mut best_peer = None;
    let mut highest_tip = Slot::from(0);

    while let Some(result) = tip_requests.next().await {
        match result {
            Ok(Ok((peer_id, tip))) => {
                if tip > highest_tip {
                    highest_tip = tip;
                    best_peer = Some(peer_id);
                }
            }
            Ok(Err(e)) => {
                error!(error = %e, "Error requesting tip from peer");
            }
            Err(_) => {
                error!("Timeout while requesting tip from peer");
            }
        }
    }

    best_peer.ok_or(SyncError::NoPeersAvailable)
}
pub async fn stream_blocks_from_peer(
    peer_id: PeerId,
    control: &mut Control,
    direction: SyncDirection,
    response_sender: Sender<(Vec<u8>, PeerId)>,
) -> Result<(), SyncError> {
    info!(peer_id = %peer_id, "Streaming blocks");

    let mut stream = sync_utils::open_stream(peer_id, control, SYNC_PROTOCOL).await?;
    sync_utils::send_data(&mut stream, &SyncRequest::Blocks { direction }).await?;

    loop {
        let message: SyncResponse = sync_utils::receive_data(&mut stream).await?;
        match message {
            SyncResponse::Block(block) => {
                if response_sender.send((block, peer_id)).await.is_err() {
                    break;
                }
            }
            SyncResponse::End => {
                info!(peer_id = %peer_id, "Received end signal");
                break;
            }
            SyncResponse::TipSlot(_) => {
                error!("Unexpected message during sync: {:?}", message);
                break;
            }
        }
    }
    stream.close().await?;

    info!(peer_id = %peer_id, "Finished streaming blocks");
    Ok(())
}
async fn request_tip_from_peer(
    peer_id: PeerId,
    mut control: Control,
) -> Result<(PeerId, Slot), SyncError> {
    info!(peer_id = %peer_id, "Requesting tip from peer");

    let mut stream = sync_utils::open_stream(peer_id, &mut control, SYNC_PROTOCOL).await?;
    sync_utils::send_data(&mut stream, &SyncRequest::TipSlot).await?;

    let message: SyncResponse = sync_utils::receive_data(&mut stream).await?;
    let SyncResponse::TipSlot(tip) = message else {
        return Err(SyncError::InvalidMessage(
            "Expected TipData message".to_owned(),
        ));
    };

    let end_message: SyncResponse = sync_utils::receive_data(&mut stream).await?;

    if matches!(end_message, SyncResponse::End) {
    } else {
        return Err(SyncError::InvalidMessage(
            "Expected End message after TipData".to_owned(),
        ));
    }

    stream.close().await?;

    info!(peer_id = %peer_id, tip = ?tip, "Received tip");
    Ok((peer_id, tip))
}

pub fn sync_after_requesting_tips(
    control: Control,
    connected_peers: &ConnectedPeers,
    local_peer_id: PeerId,
    direction: SyncDirection,
    response_sender: Sender<(Vec<u8>, PeerId)>,
) -> BoxFuture<'static, Result<(), SyncError>> {
    let peers = connected_peers.all_peers();
    async move {
        match direction {
            SyncDirection::Forward { .. } => {
                // TODO: use start_slot to filter out peers that aren't ahead of us.
                let best_peer =
                    select_best_peer_for_sync(control.clone(), peers, local_peer_id).await?;

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
