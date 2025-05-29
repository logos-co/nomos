use nomos_libp2p::cryptarchia_sync::{Block, BlocksResponse, Event, HeaderId};
use tokio::sync::mpsc;

use crate::backends::{libp2p, libp2p::swarm::SwarmHandler};

#[derive(Debug, Clone)]
pub enum ChainSyncEvent {
    ProvideBlocks {
        info: DownloadBlocksInfo,
        blocks_stream: mpsc::Sender<Block>,
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
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks,
                },
                blocks_stream,
            }),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ChainSyncCommand {
    RequestBlocks {
        info: DownloadBlocksInfo,
        blocks_stream: mpsc::Sender<BlocksResponse>,
    },
}

/// A set of block identifiers the syncing peer already knows.
#[derive(Debug, Clone)]
pub struct DownloadBlocksInfo {
    /// Return blocks up to `target_block` if specified.
    pub target_block: Option<HeaderId>,
    /// The latest block at the tip of the local chain.
    pub local_tip: HeaderId,
    /// The latest immutable block.
    pub latest_immutable_block: HeaderId,
    /// The list of additional blocks that the requester has.
    pub additional_blocks: Vec<HeaderId>,
}

impl SwarmHandler {
    pub(super) fn handle_chainsync_command(&mut self, command: ChainSyncCommand) {
        match command {
            ChainSyncCommand::RequestBlocks {
                info,
                blocks_stream,
            } => {
                if let Err(e) = self.swarm.start_blocks_download(
                    info.target_block,
                    info.local_tip,
                    info.latest_immutable_block,
                    info.additional_blocks,
                    blocks_stream,
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
