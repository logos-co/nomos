use std::fmt::{Debug, Display};

use overwatch::{
    overwatch::handle::OverwatchHandle,
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};

pub struct SystemSig<RuntimeServiceId> {
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
}

impl<RuntimeServiceId> SystemSig<RuntimeServiceId>
where
    RuntimeServiceId: Debug + Display + Sync,
{
    async fn ctrl_c_signal_received(overwatch_handle: &OverwatchHandle<RuntimeServiceId>) {
        overwatch_handle.stop_all_services().await;
        overwatch_handle.shutdown().await;
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
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        Ok(Self {
            service_resources_handle,
        })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self {
            service_resources_handle,
        } = self;
        let ctrl_c = async_ctrlc::CtrlC::new()?;

        // Wait for the Ctrl-C signal
        ctrl_c.await;
        Self::ctrl_c_signal_received(&service_resources_handle.overwatch_handle).await;

        Ok(())
    }
}
