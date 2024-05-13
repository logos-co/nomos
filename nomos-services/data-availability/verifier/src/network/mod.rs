pub mod adapters;

use futures::Stream;
use nomos_network::backends::NetworkBackend;
use nomos_network::NetworkService;
use overwatch_rs::services::relay::OutboundRelay;
use overwatch_rs::services::ServiceData;
use tokio::sync::oneshot;

#[async_trait::async_trait]
pub trait NetworkAdapter {
    type Backend: NetworkBackend + 'static;
    type Settings;

    type Blob;
    type Attestation;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<<NetworkService<Self::Backend> as ServiceData>::Message>,
    ) -> Self;

    async fn blob_stream(
        &self,
    ) -> Box<dyn Stream<Item = (Self::Blob, oneshot::Sender<Self::Attestation>)> + Unpin + Send>;
}
