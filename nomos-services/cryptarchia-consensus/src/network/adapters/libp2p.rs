use std::{hash::Hash, marker::PhantomData};

use cryptarchia_engine::Slot;
use cryptarchia_sync_network::behaviour::SyncDirection;
use nomos_core::{block::AbstractBlock, header::HeaderId, wire};
use nomos_network::{
    NetworkMsg, NetworkService,
    backends::libp2p::{
        Command, Event, EventKind, Libp2p, PeerId, PubSubCommand::Subscribe,
        SyncingCommand::StartSync,
    },
};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio_stream::{
    StreamExt,
    wrappers::{BroadcastStream, UnboundedReceiverStream, errors::BroadcastStreamRecvError},
};

use crate::{
    messages::NetworkMessage,
    network::{BoxedStream, NetworkAdapter, SyncRequest},
};

type Relay<T, RuntimeServiceId> =
    OutboundRelay<<NetworkService<T, RuntimeServiceId> as ServiceData>::Message>;

#[derive(Clone)]
pub struct LibP2pAdapter<Block, RuntimeServiceId>
where
    Block: Clone + Eq + Hash,
{
    network_relay:
        OutboundRelay<<NetworkService<Libp2p, RuntimeServiceId> as ServiceData>::Message>,
    _phantom_tx: PhantomData<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibP2pAdapterSettings {
    pub topic: String,
}

impl<Block, RuntimeServiceId> LibP2pAdapter<Block, RuntimeServiceId>
where
    Block: Clone + Eq + Hash + Serialize,
{
    async fn subscribe(relay: &Relay<Libp2p, RuntimeServiceId>, topic: &str) {
        if let Err((e, _)) = relay
            .send(NetworkMsg::Process(Command::PubSub(Subscribe(
                topic.into(),
            ))))
            .await
        {
            tracing::error!("error subscribing to {topic}: {e}");
        };
    }
}

#[async_trait::async_trait]
impl<Block, RuntimeServiceId> NetworkAdapter<RuntimeServiceId>
    for LibP2pAdapter<Block, RuntimeServiceId>
where
    Block: AbstractBlock + Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
{
    type Backend = Libp2p;
    type Settings = LibP2pAdapterSettings;
    type Block = Block;

    async fn new(settings: Self::Settings, network_relay: Relay<Libp2p, RuntimeServiceId>) -> Self {
        let relay = network_relay.clone();
        Self::subscribe(&relay, settings.topic.as_str()).await;
        tracing::debug!("Starting up...");
        // this wait seems to be helpful in some cases since we give the time
        // to the network to establish connections before we start sending messages
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        Self {
            network_relay,
            _phantom_tx: PhantomData,
        }
    }

    async fn blocks_stream(&self) -> Result<BoxedStream<Block>, DynError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::Subscribe {
                kind: EventKind::Message,
                sender,
            })
            .await
        {
            return Err(Box::new(e));
        }
        Ok(Box::new(
            BroadcastStream::new(receiver.await.map_err(Box::new)?).filter_map(|message| {
                match message {
                    Ok(Event::Message(message)) => {
                        wire::deserialize::<NetworkMessage<Block>>(&message.data).map_or_else(
                            |_| {
                                tracing::debug!("unrecognized gossipsub message");
                                None
                            },
                            |msg: NetworkMessage<_>| match msg {
                                NetworkMessage::Block(block) => {
                                    tracing::debug!("received block {:?}", block.id());
                                    Some(block)
                                }
                                NetworkMessage::SyncRequest(_) => {
                                    tracing::debug!("unrecognized message");
                                    None
                                }
                            },
                        )
                    }
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        tracing::error!("lagged messages: {n}");
                        None
                    }
                    _ => None,
                }
            }),
        ))
    }

    async fn sync_requests_stream(&self) -> Result<BoxedStream<SyncRequest>, DynError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::Subscribe {
                kind: EventKind::SyncRequest,
                sender,
            })
            .await
        {
            return Err(Box::new(e));
        }
        Ok(Box::new(
            BroadcastStream::new(receiver.await.map_err(Box::new)?).filter_map(|message| {
                match message {
                    Ok(Event::IncomingSyncRequest {
                        kind,
                        reply_channel,
                    }) => Some(SyncRequest {
                        kind,
                        reply_channel,
                    }),
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        tracing::error!("lagged messages: {n}");
                        None
                    }
                    _ => None,
                }
            }),
        ))
    }
}

#[async_trait::async_trait]
impl<Block, RuntimeServiceId> cryptarchia_sync::adapter::BlockFetcher
    for LibP2pAdapter<Block, RuntimeServiceId>
where
    Block: AbstractBlock + Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
{
    type Block = Block;
    type ProviderId = PeerId;

    async fn fetch_blocks_from_slot(
        &self,
        start_slot: Slot,
    ) -> Result<
        BoxedStream<(Self::Block, Self::ProviderId)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let command = Command::Sync(StartSync {
            direction: SyncDirection::Forward { slot: start_slot },
            response_sender: sender,
        });
        if let Err((e, _)) = self.network_relay.send(NetworkMsg::Process(command)).await {
            return Err(Box::new(e));
        }

        let stream = UnboundedReceiverStream::new(receiver).filter_map(|(block, provider_id)| {
            Some((wire::deserialize(&block).unwrap(), provider_id))
        });

        Ok(Box::new(stream))
    }
    async fn fetch_chain_backward(
        &self,
        tip: HeaderId,
        provider_id: Self::ProviderId,
    ) -> Result<BoxedStream<Block>, Box<dyn std::error::Error + Send + Sync>> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

        self.network_relay
            .send(NetworkMsg::Process(Command::Sync(StartSync {
                direction: SyncDirection::Backward {
                    start_block: tip,
                    peer: provider_id,
                },
                response_sender: sender,
            })))
            .await
            .unwrap();

        let stream = UnboundedReceiverStream::new(receiver)
            .filter_map(|(block, _)| Some(wire::deserialize(&block).unwrap()));

        Ok(Box::new(stream))
    }
}
