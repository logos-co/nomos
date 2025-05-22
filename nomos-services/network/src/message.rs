use std::fmt::Debug;

use tokio::sync::{broadcast, oneshot};

use crate::backends::NetworkBackend;

#[derive(Debug)]
pub enum NetworkMsg<Payload, EventKind, NetworkEvent> {
    Process(Payload),
    Subscribe {
        kind: EventKind,
        sender: oneshot::Sender<broadcast::Receiver<NetworkEvent>>,
    },
}

pub type BackendNetworkMsg<Backend, RuntimeServiceId> = NetworkMsg<
    <Backend as NetworkBackend<RuntimeServiceId>>::Message,
    <Backend as NetworkBackend<RuntimeServiceId>>::EventKind,
    <Backend as NetworkBackend<RuntimeServiceId>>::NetworkEvent,
>;
