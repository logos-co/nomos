use nomos_sdp::backends::SdpBackendError;
use nomos_sdp_core::{Declaration, Locator, ServiceType};
use tokio::sync::broadcast::Receiver;

pub mod producer;

pub struct DeclarationEvent {
    // todo: implement in sdp service
}

pub trait SdpAdapter {
    fn subscribe_declarations(&self) -> Receiver<DeclarationEvent>;
}
