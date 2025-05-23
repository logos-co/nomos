use std::fmt::{Debug, Display};

use futures::stream::StreamExt as _;
use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;

pub struct SystemSig<RuntimeServiceId> {
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<RuntimeServiceId> SystemSig<RuntimeServiceId>
where
    RuntimeServiceId: Debug + Display + Sync,
{
    async fn ctrlc_signal_received(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) {
        overwatch_handle.kill().await;
    }
}

impl<RuntimeServiceId> ServiceData for SystemSig<RuntimeServiceId> {
    const SERVICE_RELAY_BUFFER_SIZE: usize = 1;
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = ();
}

#[async_trait::async_trait]
impl<RuntimeServiceId> ServiceCore<RuntimeServiceId> for SystemSig<RuntimeServiceId>
where
    RuntimeServiceId: Debug + Display + Sync + Send + AsServiceId<Self>,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _init_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self { service_state })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self { service_state } = self;
        let mut ctrlc = async_ctrlc::CtrlC::new()?;
        let mut lifecycle_stream = service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                () = &mut ctrlc => {
                    Self::ctrlc_signal_received(&service_state.overwatch_handle).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if  lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
