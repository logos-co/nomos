use std::fmt::Display;

use async_trait::async_trait;
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use tokio::sync::{broadcast, oneshot};
use tracing::{error, info};

const BROADCAST_CHANNEL_SIZE: usize = 128;

pub struct BlockBroadcastService<Block, RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    new_blocks: broadcast::Sender<Block>,
}

#[derive(Debug)]
pub enum BlockBroadcastMsg<Block> {
    BroadcastNewBlock(Block),
    SubscribeToNewBlocks {
        result_sender: oneshot::Sender<broadcast::Receiver<Block>>,
    },
}

impl<Block, RuntimeServiceId> ServiceData for BlockBroadcastService<Block, RuntimeServiceId> {
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = BlockBroadcastMsg<Block>;
}

#[async_trait]
impl<Block, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for BlockBroadcastService<Block, RuntimeServiceId>
where
    Block: Clone + Send,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let (new_blocks, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);

        Ok(Self {
            service_resources_handle,
            new_blocks,
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
                BlockBroadcastMsg::BroadcastNewBlock(block) => {
                    if let Err(err) = self.new_blocks.send(block) {
                        error!("Could not send to new blocks channel: {err}");
                    }
                }
                BlockBroadcastMsg::SubscribeToNewBlocks { result_sender } => {
                    if let Err(err) = result_sender.send(self.new_blocks.subscribe()) {
                        error!("Could not subscribe to new blocks channel: {err:?}");
                    }
                }
            }
        }

        Ok(())
    }
}
