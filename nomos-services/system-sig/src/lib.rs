use futures::stream::StreamExt;
use overwatch_rs::{
    overwatch::handle::OverwatchHandle,
    services::{
        relay::NoMessage,
        state::{NoOperator, NoState},
        ServiceCore, ServiceData, ServiceId,
    },
    DynError, OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;

pub struct SystemSig {
    service_state: OpaqueServiceStateHandle<Self>,
}

impl SystemSig {
    async fn ctrlc_signal_received(overwatch_handle: &OverwatchHandle) {
        overwatch_handle.kill().await;
    }
}

impl ServiceData for SystemSig {
    const SERVICE_ID: ServiceId = "SystemSig";
    const SERVICE_RELAY_BUFFER_SIZE: usize = 1;
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State, Self::Settings>;
    type Message = NoMessage;
}

#[async_trait::async_trait]
impl ServiceCore for SystemSig {
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self { service_state })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self { service_state } = self;
        let mut ctrlc = async_ctrlc::CtrlC::new()?;
        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        #[expect(clippy::redundant_pub_crate)]
        loop {
            tokio::select! {
                () = &mut ctrlc => {
                    Self::ctrlc_signal_received(&service_state.overwatch_handle).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if  lifecycle::should_stop_service::<Self>(&msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
