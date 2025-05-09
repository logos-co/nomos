use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

use adapters::SdpAdapter;
use async_trait::async_trait;
use backends::{MembershipBackend, MembershipBackendSettings};
use futures::StreamExt;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;

mod adapters;
pub mod backends;

#[derive(Debug)]
pub enum MembershipMessage {
    GetSnapshotAt {
        reply_channel: tokio::sync::oneshot::Sender<Result<(), DynError>>, /* todo: replace with
                                                                            * actual type */
        index: i32,
        service_type: nomos_sdp_core::ServiceType,
    },
}

pub struct MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend,
    S: SdpAdapter,
{
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<B, S, RuntimeServiceId> ServiceData for MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend,
    S: SdpAdapter,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = MembershipMessage;
}

#[async_trait]
impl<B, S, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for MembershipService<B, S, RuntimeServiceId>
where
    B: MembershipBackend + Send + Sync + 'static,
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
        //todo: load settings from config
        let settings = MembershipBackendSettings {
            settings_per_service: HashMap::from([(
                nomos_sdp_core::ServiceType::DataAvailability,
                backends::SnapshotSettings::Block(10),
            )]),
        };
        Ok(Self {
            backend: B::init(settings),
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
