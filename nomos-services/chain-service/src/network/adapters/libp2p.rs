use std::{collections::HashSet, marker::PhantomData};

use cryptarchia_sync::GetTipResponse;
use futures::TryStreamExt as _;
use nomos_core::{block::Block, header::HeaderId, wire};
use nomos_network::{
    backends::libp2p::{ChainSyncCommand, Command, Libp2p, PeerId, PubSubCommand::Subscribe},
    message::{ChainSyncEvent, NetworkMsg},
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::oneshot;
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
    Tx: Clone + Eq,
    BlobCert: Clone + Eq,
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
    Tx: Clone + Eq + Serialize,
    BlobCert: Clone + Eq + Serialize,
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
    Tx: Serialize + DeserializeOwned + Clone + Eq + Send + Sync + 'static,
    BlobCert: Serialize + DeserializeOwned + Clone + Eq + Send + Sync + 'static,
{
    type Backend = Libp2p;
    type Settings = LibP2pAdapterSettings;
    type PeerId = PeerId;
    type Block = Block<Tx, BlobCert>;

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

    async fn blocks_stream(&self) -> Result<BoxedStream<Self::Block>, DynError> {
        let (sender, receiver) = oneshot::channel();
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

    async fn chainsync_events_stream(&self) -> Result<BoxedStream<ChainSyncEvent>, DynError> {
        let (sender, receiver) = oneshot::channel();

        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::SubscribeToChainSync { sender })
            .await
        {
            return Err(Box::new(e));
        }

        let stream = receiver.await.map_err(Box::new)?;
        Ok(Box::new(stream.filter_map(|event| {
            event
                .map_err(|e| tracing::error!("lagged messages: {e}"))
                .ok()
        })))
    }

    async fn request_tip(&self, peer: Self::PeerId) -> Result<GetTipResponse, DynError> {
        let (reply_sender, receiver) = oneshot::channel();
        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::Process(Command::ChainSync(
                ChainSyncCommand::RequestTip { peer, reply_sender },
            )))
            .await
        {
            return Err(Box::new(e));
        }

        let result = receiver.await.map_err(|e| Box::new(e) as DynError)?;
        result.map_err(|e| Box::new(e) as DynError)
    }

    async fn request_blocks_from_peer(
        &self,
        peer: Self::PeerId,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: HashSet<HeaderId>,
    ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError> {
        let (reply_sender, receiver) = oneshot::channel();
        if let Err((e, _)) = self
            .network_relay
            .send(NetworkMsg::Process(Command::ChainSync(
                ChainSyncCommand::DownloadBlocks {
                    peer,
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks,
                    reply_sender,
                },
            )))
            .await
        {
            return Err(Box::new(e));
        }

        let stream = receiver.await?;
        let stream = stream.map_err(|e| Box::new(e) as DynError).map(|result| {
            let block = result?;
            wire::deserialize(&block).map_err(|e| Box::new(e) as DynError)
        });

        Ok(Box::new(stream))
    }
}
