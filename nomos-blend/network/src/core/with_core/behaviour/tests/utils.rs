use core::{fmt::Debug, ops::RangeInclusive, time::Duration};
use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use futures::{select, Stream, StreamExt as _};
use libp2p::{identity::Keypair, PeerId, Swarm};
use libp2p_swarm_test::SwarmExt as _;
use nomos_libp2p::{NetworkBehaviour, SwarmEvent};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use crate::core::{
    tests::utils::PROTOCOL_NAME,
    with_core::behaviour::{Behaviour, Event, IntervalStreamProvider},
};

#[derive(Clone)]
pub struct IntervalProvider(Duration, RangeInclusive<u64>);

impl IntervalStreamProvider for IntervalProvider {
    type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let range = self.1.clone();
        Box::new(IntervalStream::new(interval(self.0)).map(move |_| range.clone()))
    }
}

#[derive(Default)]
pub struct IntervalProviderBuilder {
    range: Option<RangeInclusive<u64>>,
}

impl IntervalProviderBuilder {
    pub fn with_range(mut self, range: RangeInclusive<u64>) -> Self {
        self.range = Some(range);
        self
    }

    pub fn build(self) -> IntervalProvider {
        IntervalProvider(Duration::from_secs(1), self.range.unwrap_or(0..=1))
    }
}

#[derive(Default)]
pub struct BehaviourBuilder {
    provider: Option<IntervalProvider>,
    local_peer_id: Option<PeerId>,
    peering_degree: Option<RangeInclusive<usize>>,
    identity: Option<Keypair>,
}

impl BehaviourBuilder {
    pub fn with_provider(mut self, provider: IntervalProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_local_peer_id(mut self, local_peer_id: PeerId) -> Self {
        assert!(
            self.identity.is_none(),
            "Cannot set local peer ID when identity keypair is also set."
        );
        self.local_peer_id = Some(local_peer_id);
        self
    }

    pub fn with_peering_degree(mut self, peering_degree: RangeInclusive<usize>) -> Self {
        self.peering_degree = Some(peering_degree);
        self
    }

    pub fn with_identity(mut self, keypair: Keypair) -> Self {
        assert!(
            self.local_peer_id.is_none(),
            "Cannot set identity when local peer ID is also set."
        );
        self.identity = Some(keypair);
        self
    }

    pub fn build(self) -> Behaviour<IntervalProvider> {
        let local_peer_id = match (self.local_peer_id, self.identity) {
            (None, None) => PeerId::random(),
            (Some(peer_id), None) => peer_id,
            (None, Some(keypair)) => keypair.public().to_peer_id(),
            (Some(_), Some(_)) => panic!("Cannot happen."),
        };
        Behaviour {
            negotiated_peers: HashMap::new(),
            connections_waiting_upgrade: HashMap::new(),
            events: VecDeque::new(),
            waker: None,
            exchanged_message_identifiers: HashMap::new(),
            observation_window_clock_provider: self
                .provider
                .unwrap_or_else(|| IntervalProviderBuilder::default().build()),
            current_membership: None,
            peering_degree: self.peering_degree.unwrap_or(1..=1),
            local_peer_id,
            protocol_name: PROTOCOL_NAME,
        }
    }
}

#[async_trait]
pub trait SwarmExt: libp2p_swarm_test::SwarmExt {
    async fn connect_and_wait_for_outbound_upgrade<T>(&mut self, other: &mut Swarm<T>)
    where
        T: NetworkBehaviour<ToSwarm: Debug> + Send;
}

#[async_trait]
impl SwarmExt for Swarm<Behaviour<IntervalProvider>> {
    async fn connect_and_wait_for_outbound_upgrade<T>(&mut self, other: &mut Swarm<T>)
    where
        T: NetworkBehaviour<ToSwarm: Debug> + Send,
    {
        self.connect(other).await;
        select! {
            swarm_event = self.select_next_some() => {
                if let SwarmEvent::Behaviour(Event::OutboundConnectionUpgradeSucceeded(peer_id)) = swarm_event {
                    if peer_id == *other.local_peer_id() {
                        return;
                    }
                }
            }
            // Drive other swarm to keep polling
            _ = other.select_next_some() => {}
        }
    }
}
