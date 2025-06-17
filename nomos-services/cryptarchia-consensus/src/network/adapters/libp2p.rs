use std::{hash::Hash, marker::PhantomData};

use nomos_core::{block::Block, wire};
use nomos_network::{
    backends::libp2p::{Command, Libp2p, PubSubCommand::Subscribe},
    message::NetworkMsg,
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio_stream::{wrappers::errors::BroadcastStreamRecvError, StreamExt as _};

use crate::{
    messages::NetworkMessage,
    network::{BoxedStream, NetworkAdapter},
};

type Relay<T, RuntimeServiceId> =
    OutboundRelay<<NetworkService<T, RuntimeServiceId> as ServiceData>::Message>;

#[derive(Clone)]
pub struct LibP2pAdapter<Tx, BlobCert, RuntimeServiceId>
where
    Tx: Clone + Eq + Hash,
    BlobCert: Clone + Eq + Hash,
{
    network_relay:
        OutboundRelay<<NetworkService<Libp2p, RuntimeServiceId> as ServiceData>::Message>,
    _phantom_tx: PhantomData<Tx>,
    _blob_cert: PhantomData<BlobCert>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibP2pAdapterSettings {
    pub topic: String,
}

impl<Tx, BlobCert, RuntimeServiceId> LibP2pAdapter<Tx, BlobCert, RuntimeServiceId>
where
    Tx: Clone + Eq + Hash + Serialize,
    BlobCert: Clone + Eq + Hash + Serialize,
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
impl<Tx, BlobCert, RuntimeServiceId> NetworkAdapter<RuntimeServiceId>
    for LibP2pAdapter<Tx, BlobCert, RuntimeServiceId>
where
    Tx: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
    BlobCert: Serialize + DeserializeOwned + Clone + Eq + Hash + Send + Sync + 'static,
{
    type Backend = Libp2p;
    type Settings = LibP2pAdapterSettings;
    type Tx = Tx;
    type BlobCertificate = BlobCert;

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
            _blob_cert: PhantomData,
        }
    }

    async fn blocks_stream(
        &self,
    ) -> Result<BoxedStream<Block<Self::Tx, Self::BlobCertificate>>, DynError> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::SubscribeToPubSub { sender })
            .await
        {
            return Err(Box::new(e));
        }
        let stream = receiver.await.map_err(Box::new)?;
        Ok(Box::new(stream.filter_map(|message| match message {
            Ok(message) => wire::deserialize(&message.data).map_or_else(
                |_| {
                    tracing::debug!("unrecognized gossipsub message");
                    None
                },
                |msg| match msg {
                    NetworkMessage::Block(block) => {
                        tracing::debug!("received block {:?}", block.header().id());
                        Some(block)
                    }
                },
            ),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                tracing::error!("lagged messages: {n}");
                None
            }
        })))
    }
}
