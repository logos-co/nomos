use futures::Stream;
use nomos_core::wire;
use nomos_network::{
    backends::libp2p::{Command, Event, EventKind, Libp2p, Message, TopicHash},
    NetworkMsg, NetworkService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use serde::{de::DeserializeOwned, Serialize};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use crate::network::NetworkAdapter;

pub struct Libp2pAdapter<Item, Key, RuntimeServiceId> {
    network_relay:
        OutboundRelay<<NetworkService<Libp2p, RuntimeServiceId> as ServiceData>::Message>,
    settings: Settings<Key, Item>,
}

#[async_trait::async_trait]
impl<Item, Key, RuntimeServiceId> NetworkAdapter<RuntimeServiceId>
    for Libp2pAdapter<Item, Key, RuntimeServiceId>
where
    Item: DeserializeOwned + Serialize + Send + Sync + 'static + Clone,
    Key: Clone + Send + Sync + 'static,
{
    type Backend = Libp2p;
    type Settings = Settings<Key, Item>;
    type Payload = Item;
    type Key = Key;

    async fn new(
        settings: Self::Settings,
        network_relay: OutboundRelay<
            <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
        >,
    ) -> Self {
        network_relay
            .send(NetworkMsg::Process(Command::Subscribe(
                settings.topic.clone(),
            )))
            .await
            .expect("Network backend should be ready");
        Self {
            network_relay,
            settings,
        }
    }
    async fn payload_stream(
        &self,
    ) -> Box<dyn Stream<Item = (Self::Key, Self::Payload)> + Unpin + Send> {
        let topic_hash = TopicHash::from_raw(self.settings.topic.clone());
        let id = self.settings.id;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.network_relay
            .send(NetworkMsg::Subscribe {
                kind: EventKind::Message,
                sender,
            })
            .await
            .expect("Network backend should be ready");
        let receiver = receiver.await.unwrap();
        Box::new(Box::pin(BroadcastStream::new(receiver).filter_map(
            move |message| match message {
                Ok(Event::Message(Message { data, topic, .. })) if topic == topic_hash => {
                    match wire::deserialize::<Item>(&data) {
                        Ok(item) => Some((id(&item), item)),
                        Err(e) => {
                            tracing::debug!("Unrecognized message: {e}");
                            None
                        }
                    }
                }
                _ => None,
            },
        )))
    }

    async fn send(&self, item: Item) {
        if let Ok(wire) = wire::serialize(&item) {
            if let Err((e, _)) = self
                .network_relay
                .send(NetworkMsg::Process(Command::Broadcast {
                    topic: self.settings.topic.clone(),
                    message: wire.into(),
                }))
                .await
            {
                tracing::error!("failed to send item to topic: {e}");
            }
        } else {
            tracing::error!("Failed to serialize item");
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings<K, V> {
    pub topic: String,
    pub id: fn(&V) -> K,
}
