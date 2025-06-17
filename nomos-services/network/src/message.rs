use std::fmt::Debug;

use tokio::sync::{broadcast, oneshot};

use crate::backends::NetworkBackend;

#[derive(Debug)]
pub enum NetworkMsg<Payload, PubSubEvent, ChainSyncEvent> {
    Process(Payload),
    SubscribeToPubSub {
        sender: oneshot::Sender<broadcast::Receiver<PubSubEvent>>,
    },
    SubscribeToChainSync {
        sender: oneshot::Sender<broadcast::Receiver<ChainSyncEvent>>,
    },
}

pub type BackendNetworkMsg<Backend, RuntimeServiceId> = NetworkMsg<
    <Backend as NetworkBackend<RuntimeServiceId>>::Message,
    <Backend as NetworkBackend<RuntimeServiceId>>::PubSubEvent,
    <Backend as NetworkBackend<RuntimeServiceId>>::ChainSyncEvent,
>;
