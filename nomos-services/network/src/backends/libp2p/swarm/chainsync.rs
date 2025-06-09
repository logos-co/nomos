use nomos_libp2p::{
    cryptarchia_sync::{DownloadBlocksInfo, Event, SerialisedBlock},
    PeerId,
};
use tokio::sync::mpsc;

use crate::backends::{libp2p, libp2p::swarm::SwarmHandler};

#[derive(Debug, Clone)]
pub enum ChainSyncEvent {
    ProvideBlocks {
        info: DownloadBlocksInfo,
        blocks_stream: mpsc::Sender<SerialisedBlock>,
    },
}

impl From<Event> for libp2p::Event {
    fn from(event: Event) -> Self {
        match event {
            Event::ProvideBlocksRequest {
                target_block,
                local_tip,
                latest_immutable_block,
                additional_blocks,
                reply_sender: blocks_stream,
            } => Self::ChainSync(ChainSyncEvent::ProvideBlocks {
                info: DownloadBlocksInfo {
                    target_block: Some(target_block),
                    local_tip,
                    latest_immutable_block,
                    additional_blocks,
                },
                blocks_stream,
            }),
            Event::ProvideTipsRequest { .. } => {
                unimplemented!()
            }
            Event::DownloadBlocksResponse { .. } => {
                unimplemented!()
            }
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ChainSyncCommand {
    RequestBlocks {
        peer: PeerId,
        info: DownloadBlocksInfo,
    },
}

impl SwarmHandler {
    pub(super) fn handle_chainsync_command(&mut self, command: ChainSyncCommand) {
        match command {
            ChainSyncCommand::RequestBlocks { peer, info } => {
                if let Err(e) = self.swarm.start_blocks_download(
                    peer,
                    info.target_block,
                    info.local_tip,
                    info.latest_immutable_block,
                    info.additional_blocks,
                ) {
                    tracing::error!("failed to request blocks download: {e:?}");
                }
            }
        }
    }

    pub(super) fn handle_chainsync_event(&self, event: Event) {
        let event: libp2p::Event = event.into();
        self.event_channels.send_chainsync_event(event);
    }
}
