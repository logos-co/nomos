pub mod libp2p;
pub mod mock;

use std::{collections::HashMap, pin::Pin};

use ::libp2p::{Multiaddr, PeerId};
use futures::Stream;
use overwatch::{overwatch::handle::OverwatchHandle, services::state::ServiceState};

use super::Debug;

#[async_trait::async_trait]
pub trait NetworkBackend<RuntimeServiceId> {
    type Settings: Clone + Debug + Send + Sync + 'static;
    type State: ServiceState<Settings = Self::Settings> + Clone + Send + Sync;
    type Message: Debug + Send + Sync + 'static;
    type EventKind: Debug + Send + Sync + 'static;
    type NetworkEvent: Debug + Send + Sync + 'static;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle<RuntimeServiceId>) -> Self;
    fn shutdown(&mut self);
    fn update_membership(&mut self, members: Vec<PeerId>, addressbook: HashMap<PeerId, Multiaddr>);

    async fn process(&self, msg: Self::Message);
    async fn subscribe(
        &mut self,
        event: Self::EventKind,
    ) -> Pin<Box<dyn Stream<Item = Self::NetworkEvent> + Send>>;
}
