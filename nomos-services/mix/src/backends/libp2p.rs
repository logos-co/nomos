use std::{io, pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use libp2p::{
    core::transport::ListenerId,
    identity::{ed25519, Keypair},
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm, SwarmBuilder, TransportError,
};
use nomos_libp2p::{secret_key_serde, DialError, DialOpts, Protocol};
use overwatch_rs::overwatch::handle::OverwatchHandle;
use rand::seq::IteratorRandom;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tokio_stream::wrappers::BroadcastStream;

use super::MixBackend;

/// A mix backend that uses the libp2p network stack.
pub struct Libp2pMixBackend {
    #[allow(dead_code)]
    task: JoinHandle<()>,
    swarm_message_sender: mpsc::Sender<MixSwarmMessage>,
    fully_unwrapped_message_sender: broadcast::Sender<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Libp2pMixBackendSettings {
    pub listening_address: Multiaddr,
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    pub membership: Vec<Multiaddr>,
    pub peering_degree: usize,
    pub num_mix_layers: usize,
}

const CHANNEL_SIZE: usize = 64;

#[async_trait]
impl MixBackend for Libp2pMixBackend {
    type Settings = Libp2pMixBackendSettings;

    fn new(config: Self::Settings, overwatch_handle: OverwatchHandle) -> Self {
        let (swarm_message_sender, swarm_message_receiver) = mpsc::channel(CHANNEL_SIZE);
        let (fully_unwrapped_message_sender, _) = broadcast::channel(CHANNEL_SIZE);

        let keypair = Keypair::from(ed25519::Keypair::from(config.node_key.clone()));
        let local_peer_id = keypair.public().to_peer_id();
        let mut swarm = MixSwarm::new(
            keypair,
            config.num_mix_layers,
            swarm_message_receiver,
            fully_unwrapped_message_sender.clone(),
        );

        swarm
            .listen_on(config.listening_address)
            .unwrap_or_else(|e| {
                panic!("Failed to listen on Mix network: {e:?}");
            });

        // Randomly select peering_degree number of peers, and dial to them
        // TODO: Consider moving the peer seelction to the nomos_mix_network::Behaviour
        config
            .membership
            .iter()
            .filter(|addr| match extract_peer_id(addr) {
                Some(peer_id) => peer_id != local_peer_id,
                None => false,
            })
            .choose_multiple(&mut rand::thread_rng(), config.peering_degree)
            .iter()
            .cloned()
            .for_each(|addr| {
                if let Err(e) = swarm.dial(addr.clone()) {
                    tracing::error!("failed to dial to {:?}: {:?}", addr, e);
                }
            });

        let task = overwatch_handle.runtime().spawn(async move {
            swarm.run().await;
        });

        Self {
            task,
            swarm_message_sender,
            fully_unwrapped_message_sender,
        }
    }

    async fn mix(&self, msg: Vec<u8>) {
        if let Err(e) = self
            .swarm_message_sender
            .send(MixSwarmMessage::Mix(msg))
            .await
        {
            tracing::error!("Failed to send message to MixSwarm: {e}");
        }
    }

    fn listen_to_fully_unwrapped_messages(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = Vec<u8>> + Send>> {
        Box::pin(
            BroadcastStream::new(self.fully_unwrapped_message_sender.subscribe())
                .filter_map(|event| async { event.ok() }),
        )
    }
}

struct MixSwarm {
    swarm: Swarm<nomos_mix_network::Behaviour>,
    num_mix_layers: usize,
    swarm_messages_receiver: mpsc::Receiver<MixSwarmMessage>,
    fully_unwrapped_messages_sender: broadcast::Sender<Vec<u8>>,
}

#[derive(Debug)]
pub enum MixSwarmMessage {
    Mix(Vec<u8>),
}

impl MixSwarm {
    fn new(
        keypair: Keypair,
        num_mix_layers: usize,
        swarm_messages_receiver: mpsc::Receiver<MixSwarmMessage>,
        fully_unwrapped_messages_sender: broadcast::Sender<Vec<u8>>,
    ) -> Self {
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_quic()
            .with_behaviour(|_| {
                nomos_mix_network::Behaviour::new(nomos_mix_network::Config {
                    transmission_rate: 1.0,
                    duplicate_cache_lifespan: 60,
                })
            })
            .expect("Mix Behaviour should be built")
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
            })
            .build();

        Self {
            swarm,
            num_mix_layers,
            swarm_messages_receiver,
            fully_unwrapped_messages_sender,
        }
    }

    fn listen_on(&mut self, addr: Multiaddr) -> Result<ListenerId, TransportError<io::Error>> {
        self.swarm.listen_on(addr)
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<(), DialError> {
        self.swarm.dial(DialOpts::from(addr))
    }

    async fn run(&mut self) {
        loop {
            tokio::select! {
                Some(msg) = self.swarm_messages_receiver.recv() => {
                    self.handle_swarm_message(msg).await;
                }
                Some(event) = self.swarm.next() => {
                    self.handle_event(event);
                }
            }
        }
    }

    async fn handle_swarm_message(&mut self, msg: MixSwarmMessage) {
        match msg {
            MixSwarmMessage::Mix(msg) => {
                tracing::debug!("Wrap msg and send it to mix network: {msg:?}");
                match nomos_mix_message::new_message(&msg, self.num_mix_layers.try_into().unwrap())
                {
                    Ok(wrapped_msg) => {
                        if let Err(e) = self.swarm.behaviour_mut().publish(wrapped_msg) {
                            tracing::error!("Failed to publish message to mix network: {e:?}");
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to wrap message: {e:?}");
                    }
                }
            }
        }
    }

    fn handle_event(&mut self, event: SwarmEvent<nomos_mix_network::Event>) {
        match event {
            SwarmEvent::Behaviour(nomos_mix_network::Event::FullyUnwrappedMessage(msg)) => {
                tracing::debug!("Received fully unwrapped message: {msg:?}");
                self.fully_unwrapped_messages_sender.send(msg).unwrap();
            }
            SwarmEvent::Behaviour(nomos_mix_network::Event::Error(e)) => {
                tracing::error!("Received error from mix network: {e:?}");
            }
            _ => {
                tracing::debug!("Received event from mix network: {event:?}");
            }
        }
    }
}

fn extract_peer_id(multiaddr: &Multiaddr) -> Option<PeerId> {
    multiaddr.iter().find_map(|protocol| {
        if let Protocol::P2p(peer_id) = protocol {
            Some(peer_id)
        } else {
            None
        }
    })
}
