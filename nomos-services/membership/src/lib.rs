use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    pin::Pin,
};

use adapters::SdpAdapter;
use async_trait::async_trait;
use backends::{MembershipBackend, MembershipBackendError};
use futures::{Stream, StreamExt as _};
use nomos_sdp_core::{Locator, ProviderId, ServiceType};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

mod adapters;
pub mod backends;

type MembershipSnapshot = HashMap<ProviderId, Vec<Locator>>;

pub type MembershipSnapshotStream =
    Pin<Box<dyn Stream<Item = MembershipSnapshot> + Send + Sync + Unpin>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendSettings<S> {
    pub backend: S,
}

pub enum MembershipMessage {
    GetSnapshotAt {
        reply_channel:
            tokio::sync::oneshot::Sender<Result<MembershipSnapshot, MembershipBackendError>>,
        index: i32,
        service_type: nomos_sdp_core::ServiceType,
    },
    Subscribe {
        service_type: nomos_sdp_core::ServiceType,
        result_sender: oneshot::Sender<MembershipSnapshotStream>,
    },
}

pub struct MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend,
    S: SdpAdapter,
    B::Settings: Clone,
{
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
    subscribe_txs: HashMap<ServiceType, broadcast::Sender<MembershipSnapshot>>,
}

impl<B, S, RuntimeServiceId> ServiceData for MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend,
    S: SdpAdapter,
    B::Settings: Clone,
{
    type Settings = BackendSettings<B::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = MembershipMessage;
}

#[async_trait]
impl<B, S, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend + Send + Sync + 'static,
    B::Settings: Clone,

    RuntimeServiceId: AsServiceId<Self>
        + AsServiceId<S::SdpService>
        + Clone
        + Display
        + Send
        + Sync
        + 'static
        + Debug,
    S: SdpAdapter + Send + Sync + 'static,
    <<S as adapters::SdpAdapter>::SdpService as overwatch::services::ServiceData>::Message: 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _initstate: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        let BackendSettings {
            backend: backend_settings,
        } = service_state.settings_reader.get_updated_settings();

        Ok(Self {
            backend: B::init(backend_settings),
            service_state,
            subscribe_txs: HashMap::new(),
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let sdp_relay = self
            .service_state
            .overwatch_handle
            .relay::<S::SdpService>()
            .await?;

        let sdp_adapter = S::new(sdp_relay);
        let mut sdp_stream = sdp_adapter
            .finalized_blocks_stream()
            .await
            .map_err(|e| match e {
                adapters::SdpAdapterError::Other(error) => error,
            })?;

        let mut lifecycle_stream = self.service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(msg) = self.service_state.inbound_relay.recv()  => {
                    match msg {
                        MembershipMessage::GetSnapshotAt { reply_channel, index, service_type } =>  {
                            let result = self.backend.get_snapshot_at(service_type,index).await;

                            if let Err(e) = reply_channel.send(result) {
                                tracing::error!("Failed to send response: {:?}", e);
                            }
                        },
                        MembershipMessage::Subscribe { service_type, result_sender } => {
                            let tx = if let Some(tx) = self.subscribe_txs.get(&service_type) {
                                tx.clone()
                            } else {
                                let (tx, _) = broadcast::channel(128);
                                self.subscribe_txs.insert(service_type, tx.clone());
                                tx
                            };

                            let stream = make_pin_broadcast_stream(tx.subscribe());
                            if result_sender.send(stream).is_err() {
                                tracing::error!("Error sending finalized updates receiver");
                            }
                        },
                                            }
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
                Some(sdp_msg) = sdp_stream.next() => {
                     match self.backend.update(sdp_msg).await.map_err(|e| {
                        tracing::error!("Failed to update backend: {:?}", e);
                     }) {
                        Ok(snapshot) => {
                            for (service_type, snapshot) in snapshot {
                                if let Some(tx) = self.subscribe_txs.get(&service_type) {
                                    if tx.send(snapshot).is_err() {
                                        tracing::error!("Error sending membership update");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to update backend: {:?}", e);
                        }
                     }
                },
            }
        }
        Ok(())
    }
}

fn make_pin_broadcast_stream(
    receiver: broadcast::Receiver<MembershipSnapshot>,
) -> MembershipSnapshotStream {
    Box::pin(BroadcastStream::new(receiver).filter_map(|res| {
        Box::pin(async move {
            match res {
                Ok(update) => Some(update),
                Err(e) => {
                    tracing::warn!("Lagging Membership subscriber: {e:?}");
                    None
                }
            }
        })
    }))
}
