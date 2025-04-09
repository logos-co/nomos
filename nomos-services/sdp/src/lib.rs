pub mod backends;

use std::fmt::Display;

use async_trait::async_trait;
use backends::SdpBackend;
use futures::StreamExt;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum SdpMessage<B: SdpBackend> {
    Process {
        block_number: B::BlockNumber,
        message: B::Message,
    },

    MarkInBlock {
        block_number: B::BlockNumber,
        result_sender: oneshot::Sender<Result<(), B::Error>>,
    },
    DiscardBlock(B::BlockNumber),
}

impl<B: SdpBackend + 'static + Send + Sync, RuntimeServiceId> ServiceData
    for SdpService<B, RuntimeServiceId>
{
    type Settings = B::Settings;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = SdpMessage<B>;
}

pub struct SdpService<B: SdpBackend + Send + Sync + 'static, RuntimeServiceId> {
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

#[async_trait]
impl<B, RuntimeServiceId> ServiceCore<RuntimeServiceId> for SdpService<B, RuntimeServiceId>
where
    B: SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _initstate: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: B::new(service_state.settings_reader.get_updated_settings()),
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let Self {
            mut backend,
            service_state:
                OpaqueServiceStateHandle::<Self, RuntimeServiceId> {
                    mut inbound_relay,
                    lifecycle_handle,
                    ..
                },
        } = self;
        let mut lifecycle_stream = lifecycle_handle.message_stream();
        let backend = &mut backend;
        loop {
            tokio::select! {
                Some(msg) = inbound_relay.recv() => {
                    Self::handle_sdp_message(msg, backend).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<B: SdpBackend + Send + Sync + 'static, RuntimeServiceId> SdpService<B, RuntimeServiceId> {
    async fn handle_sdp_message(msg: SdpMessage<B>, backend: &mut B) {
        match msg {
            SdpMessage::Process {
                block_number,
                message,
            } => {
                if let Err(e) = backend.process_sdp_message(block_number, message).await {
                    tracing::error!("Error processing SDP message: {}", e);
                }
            }
            SdpMessage::MarkInBlock {
                block_number,
                result_sender,
            } => {
                let result = backend.mark_in_block(block_number).await;
                let result = result_sender.send(result);
                if let Err(e) = result {
                    tracing::error!("Error sending result: {:?}", e);
                }
            }
            SdpMessage::DiscardBlock(block_number) => {
                backend.discard_block(block_number);
            }
        }
    }
}
