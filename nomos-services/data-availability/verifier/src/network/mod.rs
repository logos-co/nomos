pub mod adapters;

use futures::Stream;
use nomos_da_network_service::{backends::NetworkBackend, NetworkService};
use overwatch::services::{relay::OutboundRelay, ServiceData};

#[async_trait::async_trait]
pub trait NetworkAdapter {
    type Backend: NetworkBackend + Send + 'static;
    type Settings;

    type Blob;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<<NetworkService<Self::Backend> as ServiceData>::Message>,
    ) -> Self;

    async fn blob_stream(&self) -> Box<dyn Stream<Item = Self::Blob> + Unpin + Send>;
}
