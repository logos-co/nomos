use async_trait::async_trait;
use overwatch::{DynError, OpaqueServiceResourcesHandle, services::ServiceData};

#[async_trait]
pub trait Mode<RuntimeServiceId> {
    type Settings;
    type Message;

    async fn run<Service>(
        service_resource_handle: OpaqueServiceResourcesHandle<Service, RuntimeServiceId>,
    ) -> Result<OpaqueServiceResourcesHandle<Service, RuntimeServiceId>, DynError>
    where
        Service: ServiceData<
                Settings: Into<Self::Settings> + Clone + Send + Sync,
                State: Send + Sync,
                Message = Self::Message,
            >;
}
