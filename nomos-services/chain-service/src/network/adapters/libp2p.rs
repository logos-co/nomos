use std::{collections::HashSet, fmt::Debug, hash::Hash, marker::PhantomData};

use cryptarchia_sync::GetTipResponse;
use futures::TryStreamExt as _;
use nomos_core::{block::Block, header::HeaderId, wire};
use nomos_network::{
    backends::libp2p::{
        ChainSyncCommand, Command, DiscoveryCommand, Libp2p, NetworkCommand, PeerId,
        PubSubCommand::Subscribe,
    },
    message::{ChainSyncEvent, NetworkMsg},
    NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use rand::{seq::index::sample, thread_rng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_stream::{wrappers::errors::BroadcastStreamRecvError, StreamExt as _};
use tracing::debug;

use crate::{
    messages::NetworkMessage,
    network::{BoxedStream, NetworkAdapter},
};

const MAX_PEERS_TO_TRY_FOR_ORPHAN_DOWNLOAD: usize = 3;

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

    async fn get_connected_peers(
        relay: &Relay<Libp2p, RuntimeServiceId>,
    ) -> Result<HashSet<PeerId>, DynError> {
        let (reply_sender, receiver) = oneshot::channel();
        if let Err((e, _)) = relay
            .send(NetworkMsg::Process(Command::Network(
                NetworkCommand::ConnectedPeers {
                    reply: reply_sender,
                },
            )))
            .await
        {
            return Err(Box::new(e));
        }

        let connected_peers = receiver.await.map_err(|e| Box::new(e) as DynError)?;
        Ok(connected_peers)
    }

    async fn get_discovered_peers(
        relay: &Relay<Libp2p, RuntimeServiceId>,
    ) -> Result<HashSet<PeerId>, DynError> {
        let (reply_sender, receiver) = oneshot::channel();
        if let Err((e, _)) = relay
            .send(NetworkMsg::Process(Command::Discovery(
                DiscoveryCommand::GetDiscoveredPeers {
                    reply: reply_sender,
                },
            )))
            .await
        {
            return Err(Box::new(e));
        }

        let discovered_peers = receiver.await.map_err(|e| Box::new(e) as DynError)?;

        Ok(discovered_peers)
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

    /// Attempts to open a stream of blocks from a locally known block to the
    /// target_block block.
    async fn request_blocks_from_peers(
        &self,
        target_block: HeaderId,
        local_tip: HeaderId,
        latest_immutable_block: HeaderId,
        additional_blocks: HashSet<HeaderId>,
    ) -> Result<BoxedStream<Result<Self::Block, DynError>>, DynError> {
        let connected_peers = Self::get_connected_peers(&self.network_relay).await?;

        // All peers we know about, including those that are not connected.
        let discovered_peers = Self::get_discovered_peers(&self.network_relay).await?;

        let peers_to_request = choose_peers_to_request_download(
            connected_peers,
            &discovered_peers,
            MAX_PEERS_TO_TRY_FOR_ORPHAN_DOWNLOAD,
        );

        for peer in peers_to_request {
            match self
                .request_blocks_from_peer(
                    peer,
                    // as the target block
                    target_block,
                    local_tip,
                    latest_immutable_block,
                    additional_blocks.clone(),
                )
                .await
            {
                Ok(stream) => {
                    debug!("Requested orphan parents from peer: {peer}");
                    return Ok(stream);
                }
                Err(err) => {
                    tracing::warn!("Failed to request orphan parents from peer {peer}: {err}");
                }
            }
        }

        Err("Failed to request orphan parents from any peer".into())
    }
}

/// Selects up to `MAX_PEERS_TO_TRY_FOR_ORPHAN_DOWNLOAD` peers to request
/// downloads from, preferring discovered peers that are not currently
/// connected. If not enough, fills from connected peers.
///
/// Returned the list of `PeerId` with discovered peers appearing before
/// connected ones(if any).
fn choose_peers_to_request_download<PeerId>(
    connected_peers: HashSet<PeerId>,
    discovered_peers: &HashSet<PeerId>,
    max: usize,
) -> Vec<PeerId>
where
    PeerId: Clone + Eq + Hash + Copy + Debug,
{
    let discovered_only: Vec<_> = discovered_peers
        .difference(&connected_peers)
        .copied()
        .collect();

    // Use Vec to keep not connected in front
    let mut selected = Vec::new();

    let discovered_only_max_to_use = discovered_only.len().min(max);

    if discovered_only_max_to_use > 0 {
        let indexes = sample(
            &mut thread_rng(),
            discovered_only.len(),
            discovered_only_max_to_use,
        );
        selected.extend(indexes.into_iter().map(|i| discovered_only[i]));
    }

    let remaining = max.saturating_sub(selected.len());
    let remaining_available = remaining.min(connected_peers.len());

    if remaining_available > 0 {
        let connected_vec: Vec<_> = connected_peers.into_iter().collect();

        let indexes = sample(&mut thread_rng(), connected_vec.len(), remaining_available);
        selected.extend(indexes.into_iter().map(|i| connected_vec[i]));
    }

    selected
}

#[cfg(test)]
mod tests {

    use super::*;

    const MAX: usize = MAX_PEERS_TO_TRY_FOR_ORPHAN_DOWNLOAD;

    #[test]
    fn returns_only_discovered_peers() {
        let connected = HashSet::from_iter(vec![[1; 32], [2; 32]]);
        let discovered = HashSet::from_iter(vec![[3; 32], [4; 32], [5; 32]]);

        let result = choose_peers_to_request_download(connected, &discovered, MAX);

        assert_eq!(result.len(), 3);
        assert!(result.contains(&[3; 32]));
        assert!(result.contains(&[4; 32]));
        assert!(result.contains(&[5; 32]));
    }

    #[test]
    fn returns_all_connected_peers_if_no_discovered_peers() {
        let connected = HashSet::from_iter(vec![[1; 32], [2; 32], [3; 32]]);
        let discovered = HashSet::new();

        let result = choose_peers_to_request_download(connected.clone(), &discovered, MAX);

        assert_eq!(result.len(), connected.len());
    }

    #[test]
    fn includes_connected_peers_if_discovered_peers_are_not_enough() {
        let connected = HashSet::from_iter(vec![[1; 32], [2; 32], [3; 32]]);
        let discovered = HashSet::from_iter(vec![[4; 32]]);

        let result = choose_peers_to_request_download(connected, &discovered, MAX);

        assert_eq!(result.len(), MAX);
        assert!(result.contains(&[4; 32]));
    }

    #[test]
    fn limits_number_of_peers_to_max_attempts() {
        let connected = (0..=MAX).map(|id| [id; 32]).collect::<HashSet<_>>();
        let discovered = HashSet::new();

        let result = choose_peers_to_request_download(connected, &discovered, MAX);
        assert_eq!(result.len(), MAX);
    }
}
