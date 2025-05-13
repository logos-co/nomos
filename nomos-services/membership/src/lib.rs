use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use adapters::SdpAdapter;
use async_trait::async_trait;
use backends::MembershipBackend;
use futures::StreamExt as _;
use nomos_sdp_core::{DeclarationUpdate, ProviderInfo};
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use serde::{Deserialize, Serialize};
use services_utils::overwatch::lifecycle;

mod adapters;
pub mod backends;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackendSettings<S> {
    pub backend: S,
}

#[derive(Debug)]
pub enum MembershipMessage {
    GetSnapshotAt {
        reply_channel: tokio::sync::oneshot::Sender<
            Result<HashMap<ProviderInfo, DeclarationUpdate>, DynError>,
        >,
        index: i32,
        service_type: nomos_sdp_core::ServiceType,
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
                        }
                    }
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
                Some(sdp_msg) = sdp_stream.next() => {
                     if let Err(e) = self.backend.update(sdp_msg).await.map_err(|e| {
                        tracing::error!("Failed to update backend: {:?}", e);
                     }) {
                        tracing::error!("Failed to process SDP message: {:?}", e);
                    }
                },
            }
        }
        Ok(())
    }
}
