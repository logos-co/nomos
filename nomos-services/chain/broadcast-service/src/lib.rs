use std::fmt::Display;

use async_trait::async_trait;
use nomos_core::{header::HeaderId, sdp::SessionUpdate};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot, watch};
use tracing::{error, info};

const BROADCAST_CHANNEL_SIZE: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockInfo {
    pub height: u64,
    pub header_id: HeaderId,
}

#[derive(Debug)]
pub enum BlockBroadcastMsg {
    BroadcastFinalizedBlock(BlockInfo),
    BroadcastBlendSession(SessionUpdate),
    BroadcastDASession(SessionUpdate),
    SubscribeToFinalizedBlocks {
        result_sender: oneshot::Sender<broadcast::Receiver<BlockInfo>>,
    },
    SubscribeBlendSession {
        result_sender: oneshot::Sender<watch::Receiver<Option<SessionUpdate>>>,
    },
    SubscribeDASession {
        result_sender: oneshot::Sender<watch::Receiver<Option<SessionUpdate>>>,
    },
}

pub struct BlockBroadcastService<RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    finalized_blocks: broadcast::Sender<BlockInfo>,
    blend_session: watch::Sender<Option<SessionUpdate>>,
    da_session: watch::Sender<Option<SessionUpdate>>,
}

impl<RuntimeServiceId> ServiceData for BlockBroadcastService<RuntimeServiceId> {
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = BlockBroadcastMsg;
}

#[async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for BlockBroadcastService<RuntimeServiceId>
where
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let (finalized_blocks, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);
        let (blend_session, _) = watch::channel(None);
        let (da_session, _) = watch::channel(None);

        Ok(Self {
            service_resources_handle,
            finalized_blocks,
            blend_session,
            da_session,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        self.service_resources_handle.status_updater.notify_ready();
        info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(msg) = self.service_resources_handle.inbound_relay.recv().await {
            match msg {
                BlockBroadcastMsg::BroadcastFinalizedBlock(block) => {
                    if let Err(err) = self.finalized_blocks.send(block) {
                        error!("Could not send to new blocks channel: {err}");
                    }
                }
                BlockBroadcastMsg::BroadcastBlendSession(session) => {
                    if let Err(err) = self.blend_session.send(Some(session)) {
                        error!("Could not send to new blocks channel: {err}");
                    }
                }
                BlockBroadcastMsg::BroadcastDASession(session) => {
                    if let Err(err) = self.da_session.send(Some(session)) {
                        error!("Could not send to new blocks channel: {err}");
                    }
                }
                BlockBroadcastMsg::SubscribeToFinalizedBlocks { result_sender } => {
                    // TODO: This naively broadcast what was sent from the chain service. In case
                    // of LIB branch change (might happend during bootstrapping), blocks should be
                    // rebroadcasted from the last common header_id.
                    if let Err(err) = result_sender.send(self.finalized_blocks.subscribe()) {
                        error!("Could not subscribe to new blocks channel: {err:?}");
                    }
                }
                BlockBroadcastMsg::SubscribeBlendSession { result_sender } => {
                    if let Err(err) = result_sender.send(self.blend_session.subscribe()) {
                        error!("Could not subscribe to blend session channel: {err:?}");
                    }
                }
                BlockBroadcastMsg::SubscribeDASession { result_sender } => {
                    if let Err(err) = result_sender.send(self.da_session.subscribe()) {
                        error!("Could not subscribe to DA session channel: {err:?}");
                    }
                }
            }
        }

        Ok(())
    }
}
