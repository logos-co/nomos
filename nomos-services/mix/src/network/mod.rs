#[cfg(feature = "libp2p")]
pub mod libp2p;

use std::fmt::Debug;

use nomos_network::backends::NetworkBackend;
use nomos_network::NetworkService;
use overwatch_rs::services::relay::OutboundRelay;
use overwatch_rs::services::ServiceData;
use serde::{de::DeserializeOwned, Serialize};

/// A trait for communicating with the network service, which is used to broadcast
/// fully unwrapped messages returned from the mix backend.
#[async_trait::async_trait]
pub trait NetworkAdapter {
    /// The network backend used by the network service.
    type Backend: NetworkBackend + 'static;
    /// Settings used to broadcast messages using the network service.
    type BroadcastSettings: Clone + Debug + Serialize + DeserializeOwned + Send + Sync + 'static;

    fn new(
        network_relay: OutboundRelay<<NetworkService<Self::Backend> as ServiceData>::Message>,
    ) -> Self;
    /// Broadcast a message to the network service using the specified broadcast settings.
    async fn broadcast(&self, message: Vec<u8>, broadcast_settings: Self::BroadcastSettings);
}
