pub mod backends;

use std::{collections::BTreeSet, fmt::Display, marker::PhantomData, pin::Pin};

use async_trait::async_trait;
use backends::{SdpBackend, SdpBackendError};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    block::BlockNumber,
    sdp::{Locator, ProviderId, ServiceType},
};
use overwatch::{
    OpaqueServiceResourcesHandle,
    services::{
        AsServiceId, ServiceCore, ServiceData,
        state::{NoOperator, NoState},
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

const BROADCAST_CHANNEL_SIZE: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclarationState {
    Active,
    Inactive,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEventUpdate {
    pub service_type: ServiceType,
    pub provider_id: ProviderId,
    pub state: DeclarationState,
    pub locators: BTreeSet<Locator>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEvent {
    pub block_number: BlockNumber,
    pub updates: Vec<BlockEventUpdate>,
}

pub type BlockUpdateStream = Pin<Box<dyn Stream<Item = BlockEvent> + Send + Sync + Unpin>>;

pub enum SdpMessage {
    ProcessNewBlock,
    ProcessLibBlock,
    Subscribe {
        result_sender: oneshot::Sender<BlockUpdateStream>,
    },
}

pub struct SdpService<Backend: SdpBackend + Send + Sync + 'static, Metadata, RuntimeServiceId>
where
    Metadata: Send + Sync + 'static,
{
    backend: PhantomData<Backend>,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    finalized_update_tx: broadcast::Sender<BlockEvent>,
}

impl<Backend, Metadata, RuntimeServiceId> ServiceData
    for SdpService<Backend, Metadata, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = SdpMessage;
}

#[async_trait]
impl<Backend, Metadata, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for SdpService<Backend, Metadata, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    Metadata: Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let (finalized_update_tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);

        Ok(Self {
            backend: PhantomData,
            service_resources_handle,
            finalized_update_tx,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        self.service_resources_handle.status_updater.notify_ready();
        tracing::info!(
            "Service '{}' is ready.",
            <RuntimeServiceId as AsServiceId<Self>>::SERVICE_ID
        );

        while let Some(msg) = self.service_resources_handle.inbound_relay.recv().await {
            match msg {
                SdpMessage::ProcessNewBlock | SdpMessage::ProcessLibBlock => {
                    todo!()
                }
                SdpMessage::Subscribe { result_sender } => {
                    let receiver = self.finalized_update_tx.subscribe();
                    let stream = make_finalized_stream(receiver);

                    if result_sender.send(stream).is_err() {
                        tracing::error!("Error sending finalized updates receiver");
                    }
                }
            }
        }

        Ok(())
    }
}

fn make_finalized_stream(receiver: broadcast::Receiver<BlockEvent>) -> BlockUpdateStream {
    Box::pin(BroadcastStream::new(receiver).filter_map(|res| {
        Box::pin(async move {
            match res {
                Ok(update) => Some(update),
                Err(e) => {
                    tracing::warn!("Lagging SDP subscriber: {e:?}");
                    None
                }
            }
        })
    }))
}
