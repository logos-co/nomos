pub mod backends;

use std::{collections::BTreeSet, fmt::Display, marker::PhantomData, pin::Pin};

use async_trait::async_trait;
use backends::{SdpBackend, SdpBackendError};
use futures::{Stream, StreamExt as _};
use nomos_core::{
    block::BlockNumber,
    sdp::{ActivityMetadata, DeclarationId, Locator, ProviderId, ServiceType},
};
use overwatch::{
    DynError, OpaqueServiceResourcesHandle,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SdpSettings {
    /// Declaration ID for this node (set after posting declaration and
    /// restarting)
    pub declaration_id: Option<DeclarationId>,
}

pub enum SdpMessage {
    ProcessNewBlock,
    ProcessLibBlock,
    Subscribe {
        result_sender: oneshot::Sender<BlockUpdateStream>,
    },

    PostDeclaration {
        service_type: ServiceType,
        locators: Vec<Locator>,
        reply_channel: oneshot::Sender<Result<DeclarationId, DynError>>,
    },
    PostActivity {
        metadata: ActivityMetadata, // DA/Blend specific metadata
    },
    PostWithdrawal {
        declaration_id: DeclarationId,
    },
}

pub struct SdpService<Backend, RuntimeServiceId> {
    backend: PhantomData<Backend>,
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    finalized_update_tx: broadcast::Sender<BlockEvent>,
    _current_declaration_id: Option<DeclarationId>,
}

impl<Backend, RuntimeServiceId> ServiceData for SdpService<Backend, RuntimeServiceId> {
    type Settings = SdpSettings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = SdpMessage;
}

#[async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for SdpService<Backend, RuntimeServiceId>
where
    Backend: SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let settings = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();

        let (finalized_update_tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);

        Ok(Self {
            _current_declaration_id: settings.declaration_id,
            backend: PhantomData,
            service_resources_handle,
            finalized_update_tx,
        })
    }

    async fn run(mut self) -> Result<(), DynError> {
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
                SdpMessage::PostActivity { metadata, .. } => {
                    tracing::debug!("todo: implement post activity {:?}", metadata);
                }
                SdpMessage::PostDeclaration { .. } => todo!("implement post declaration"),
                SdpMessage::PostWithdrawal { .. } => todo!("implement post withdrawal"),
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
