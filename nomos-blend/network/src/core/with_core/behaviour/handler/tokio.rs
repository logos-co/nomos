pub use std::{num::NonZeroU64, ops::RangeInclusive, time::Duration};

pub use nomos_utils::math::NonNegativeF64;
pub use tokio_stream::StreamExt as _;

use crate::core::with_core::behaviour::IntervalStreamProvider;

#[derive(Clone)]
/// Provider of a stream of observation windows used by the Blend connection
/// monitor to evaluate peers.
///
/// At each interval, it returns the [min,max] (inclusive) range of expected
/// messages from the peer, as per the specification.
pub struct ObservationWindowTokioIntervalProvider {
    pub round_duration_seconds: NonZeroU64,
    pub maximal_delay_rounds: NonZeroU64,
    pub blending_ops_per_message: u64,
    pub normalization_constant: NonNegativeF64,
    pub membership_size: NonZeroU64,
    pub rounds_per_observation_window: NonZeroU64,
    pub minimum_messages_coefficient: NonZeroU64,
}

impl ObservationWindowTokioIntervalProvider {
    fn calculate_expected_message_range(&self) -> RangeInclusive<u64> {
        // TODO: Remove unsafe arithmetic operations
        let mu = ((self.maximal_delay_rounds.get() as f64
            * self.blending_ops_per_message as f64
            * self.normalization_constant.get())
            / self.membership_size.get() as f64)
            .ceil() as u64;
        (mu * self.minimum_messages_coefficient.get())
            ..=(mu * self.rounds_per_observation_window.get())
    }
}

impl IntervalStreamProvider for ObservationWindowTokioIntervalProvider {
    type IntervalStream =
        Box<dyn futures::Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
    type IntervalItem = RangeInclusive<u64>;

    fn interval_stream(&self) -> Self::IntervalStream {
        let expected_message_range = self.calculate_expected_message_range();
        Box::new(
            tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
                Duration::from_secs(
                    (self.rounds_per_observation_window.get()) * self.round_duration_seconds.get(),
                ),
            ))
            .map(move |_| expected_message_range.clone()),
        )
    }
}

#[cfg(test)]
mod test {
    use std::{ops::RangeInclusive, time::Duration};

    use futures::Stream;
    use libp2p::{
        futures::StreamExt as _,
        identity::Keypair,
        swarm::{dummy, NetworkBehaviour, SwarmEvent},
        Multiaddr, PeerId, Swarm, SwarmBuilder,
    };
    use nomos_blend_message::crypto::Ed25519PrivateKey;
    use nomos_blend_scheduling::membership::{Membership, Node};
    use tokio::{select, time::sleep};

    use crate::core::with_core::{
        behaviour::{Behaviour, Config, Event, IntervalStreamProvider, NegotiatedPeerState},
        error::Error,
    };

    struct TestTokioIntervalStreamProvider(Duration, RangeInclusive<u64>);

    impl IntervalStreamProvider for TestTokioIntervalStreamProvider {
        type IntervalStream = Box<dyn Stream<Item = RangeInclusive<u64>> + Send + Unpin + 'static>;
        type IntervalItem = RangeInclusive<u64>;

        fn interval_stream(&self) -> Self::IntervalStream {
            let interval = self.0;
            let range = self.1.clone();
            Box::new(
                tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(interval))
                    .map(move |_| range.clone()),
            )
        }
    }

    /// Check that a published messsage arrives in the peers successfully.
    #[tokio::test]
    async fn behaviour() {
        // Initialize two swarms that support the blend protocol.
        let (mut nodes, mut keypairs) = nodes(2, 8090);
        let node1_addr = nodes.next().unwrap().address;
        let mut swarm1 = new_blend_swarm(
            keypairs.next().unwrap(),
            node1_addr.clone(),
            Duration::from_secs(5),
            None,
            None,
            None,
        );
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            nodes.next().unwrap().address,
            Duration::from_secs(5),
            None,
            None,
            None,
        );
        swarm2.dial(node1_addr).unwrap();

        // Swamr2 publishes a message.
        let task = async {
            let msg = vec![1; 10];
            let mut msg_published = false;
            let mut publish_try_interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                select! {
                    // Try to publish a message until it succeeds.
                    // (It will fail until swarm2 is connected to swarm1 successfully.)
                    _ = publish_try_interval.tick() => {
                        if !msg_published {
                            msg_published = swarm2.behaviour_mut().publish(&msg).is_ok();
                        }
                    }
                    // Proceed swarm1
                    event = swarm1.select_next_some() => {
                        if let SwarmEvent::Behaviour(Event::Message(received_msg, _)) = event {
                            assert_eq!(received_msg, msg);
                            break;
                        }
                    }
                    // Proceed swarm2
                    _ = swarm2.select_next_some() => {}
                }
            }
        };

        // Expect for the task to be completed within 30 seconds.
        assert!(tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .is_ok());
    }

    /// If the peer doesn't support the blend protocol, the message should
    /// not be forwarded to the peer.
    #[test_log::test(tokio::test)]
    async fn peer_not_support_blend_protocol() {
        // Only swarm2 supports the blend protocol.
        let (mut nodes, mut keypairs) = nodes(2, 8190);
        let node1_addr = nodes.next().unwrap().address;
        let mut swarm1 = new_dummy_swarm(keypairs.next().unwrap(), node1_addr.clone());
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            nodes.next().unwrap().address,
            Duration::from_secs(5),
            None,
            None,
            None,
        );
        swarm2.dial(node1_addr).unwrap();

        // Expect all publish attempts to fail with [`Error::NoPeers`]
        // because swarm2 doesn't have any peers that support the blend protocol.
        let msg = vec![1; 10];
        let mut publish_try_interval = tokio::time::interval(Duration::from_secs(1));
        let mut publish_try_count = 0;
        loop {
            select! {
                _ = publish_try_interval.tick() => {
                    assert!(matches!(swarm2.behaviour_mut().publish(&msg), Err(Error::NoPeers)));
                    publish_try_count += 1;
                    if publish_try_count >= 10 {
                        break;
                    }
                }
                _ = swarm1.select_next_some() => {}
                _ = swarm2.select_next_some() => {}
            }
        }
    }

    #[test_log::test(tokio::test)]
    async fn detect_spammy_peer() {
        // Init two swarms with connection monitoring enabled.
        let (mut nodes, mut keypairs) = nodes(2, 8290);
        let mut swarm1 = new_blend_swarm(
            keypairs.next().unwrap(),
            nodes.next().unwrap().address,
            Duration::from_secs(5),
            Some(0..=0),
            None,
            None,
        );
        let node2_addr = nodes.next().unwrap().address;
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            node2_addr.clone(),
            Duration::from_secs(5),
            Some(0..=0),
            None,
            None,
        );
        swarm1.dial(node2_addr).unwrap();

        // Swarm2 sends a message to Swarm1, even though `expected_messages` is
        // 0. Then, Swarm1 should close the connection with Swarm2 which is flagged as
        //    spammy.
        let task = async {
            let mut num_events_waiting = 2;
            let mut msg_published = false;
            let mut publish_try_interval = tokio::time::interval(Duration::from_millis(10));
            loop {
                if num_events_waiting == 0 {
                    break;
                }

                select! {
                    _ = publish_try_interval.tick() => {
                        if !msg_published {
                            msg_published = swarm2.behaviour_mut().publish(&[1; 10]).is_ok();
                        }
                    }
                    event = swarm1.select_next_some() => {
                        match event {
                            SwarmEvent::Behaviour(Event::PeerDisconnected(peer_id, _, NegotiatedPeerState::Spammy)) => {
                                assert_eq!(peer_id, *swarm2.local_peer_id());
                                num_events_waiting -= 1;
                            },
                            // We expect that the Swarm1 closes the connection proactively.
                            SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                                assert_eq!(peer_id, *swarm2.local_peer_id());
                                assert_eq!(num_established, 0);
                                assert!(swarm1.connected_peers().next().is_none());
                                num_events_waiting -= 1;
                            }
                            _ => {}
                        }
                    }
                    _ = swarm2.select_next_some() => {}
                }
            }
        };

        // Expect for the task to be completed in time
        assert!(tokio::time::timeout(Duration::from_secs(6), task)
            .await
            .is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn detect_unhealthy_peer() {
        // Init two swarms with connection monitoring enabled.
        let (mut nodes, mut keypairs) = nodes(2, 8390);
        let node1_addr = nodes.next().unwrap().address;
        let mut swarm1 = new_blend_swarm(
            keypairs.next().unwrap(),
            node1_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            None,
        );
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            nodes.next().unwrap().address,
            Duration::from_secs(5),
            Some(1..=1),
            None,
            None,
        );
        swarm2.dial(node1_addr).unwrap();

        // Swarms don't send anything, even though `expected_messages` is 1.
        // Then, both should detect the other as unhealthy.
        // Swarms shouldn't close the connection of the unhealthy peers.
        let task = async {
            let mut num_events_waiting = 2;
            loop {
                if num_events_waiting == 0 {
                    break;
                }

                select! {
                    event = swarm1.select_next_some() => {
                        if let SwarmEvent::Behaviour(Event::UnhealthyPeer(peer_id, _)) = event {
                            assert_eq!(peer_id, *swarm2.local_peer_id());
                            num_events_waiting -= 1;
                        }
                    }
                    event = swarm2.select_next_some() => {
                        if let SwarmEvent::Behaviour(Event::UnhealthyPeer(peer_id, _)) = event {
                            assert_eq!(peer_id, *swarm1.local_peer_id());
                            num_events_waiting -= 1;
                        }
                    }
                }
            }

            assert_eq!(swarm1.behaviour().num_healthy_peers(), 0);
            assert_eq!(swarm1.connected_peers().count(), 1);
            assert_eq!(swarm2.behaviour().num_healthy_peers(), 0);
            assert_eq!(swarm2.connected_peers().count(), 1);
        };

        // Expect for the task to be completed in time
        assert!(tokio::time::timeout(Duration::from_secs(6), task)
            .await
            .is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn outgoing_connection_limit() {
        // All swarms can maintain a single connection, either outgoing (since min
        // outgoing is 0) or incoming (since max total is 1 and min outgoing is 0).
        let (mut nodes, mut keypairs) = nodes(3, 8490);
        let node1_addr = nodes.next().unwrap().address;
        let mut swarm1 = new_blend_swarm(
            keypairs.next().unwrap(),
            node1_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(0..=1),
        );
        let node2_addr = nodes.next().unwrap().address;
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            node2_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(0..=1),
        );
        let mut swarm3 = new_blend_swarm(
            keypairs.next().unwrap(),
            nodes.next().unwrap().address,
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(0..=1),
        );
        // Will succeed
        swarm3.dial(node1_addr).unwrap();
        // Small delay to make sure node1 dial attempt happens before node 2.
        sleep(Duration::from_millis(500)).await;
        // Will fail
        swarm3.dial(node2_addr).unwrap();

        let mut num_events_waiting: u8 = 4;

        loop {
            if num_events_waiting == 0 {
                break;
            }
            select! {
                event = swarm1.select_next_some() => {
                    if let SwarmEvent::Behaviour(Event::IncomingConnectionRequestAccepted(peer_id, _)) = event {
                        assert_eq!(peer_id, *swarm3.local_peer_id());
                        num_events_waiting -= 1;
                    }
                }
                event = swarm2.select_next_some() => {
                    if let SwarmEvent::Behaviour(Event::IncomingConnectionRequestAccepted(peer_id, _)) = event {
                        assert_eq!(peer_id, *swarm3.local_peer_id());
                        num_events_waiting -= 1;
                    }
                }
                event = swarm3.select_next_some() => {
                    match event {
                        SwarmEvent::Behaviour(Event::OutgoingConnectionRequestAccepted(peer_id, _)) => {
                            assert_eq!(peer_id, *swarm1.local_peer_id());
                            num_events_waiting -= 1;
                        }
                        SwarmEvent::Behaviour(Event::OutgoingConnectionRequestDiscarded(peer_id, _)) => {
                            assert_eq!(peer_id, *swarm2.local_peer_id());
                            num_events_waiting -= 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    #[test_log::test(tokio::test)]
    async fn incoming_connection_limit() {
        let (mut nodes, mut keypairs) = nodes(3, 8590);
        let node1_addr = nodes.next().unwrap().address;
        // Swarm 1 can have 1 outgoing and 1 incoming connection.
        let mut swarm1 = new_blend_swarm(
            keypairs.next().unwrap(),
            node1_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(1..=2),
        );
        // Node 2 can have a single outgoing connection.
        let node2_addr = nodes.next().unwrap().address;
        let mut swarm2 = new_blend_swarm(
            keypairs.next().unwrap(),
            node2_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(1..=1),
        );
        // Node 3 (the one being tested) can have 1 outgoing and 1 incoming connection.
        let node3_addr = nodes.next().unwrap().address;
        let mut swarm3 = new_blend_swarm(
            keypairs.next().unwrap(),
            node3_addr.clone(),
            Duration::from_secs(5),
            Some(1..=1),
            None,
            Some(1..=2),
        );
        // Will succeed. It will take 1 outgoing slot for swarm 1 and 1 incoming slot
        // for swarm 3.
        swarm1.dial(node3_addr.clone()).unwrap();
        sleep(Duration::from_millis(500)).await;
        // Will succeed. It will take 1 outgoing slot for swarm 3 and 1 incoming slot
        // for swarm 1.
        swarm3.dial(node1_addr).unwrap();
        sleep(Duration::from_millis(500)).await;
        // Will fail. Swarm 3 has all incoming slots taken.
        swarm2.dial(node3_addr).unwrap();

        let mut num_events_waiting: u8 = 3;

        loop {
            if num_events_waiting == 0 {
                break;
            }
            select! {
                                        event = swarm1.select_next_some() => {
                                            if let SwarmEvent::Behaviour(Event::IncomingConnectionRequestAccepted(peer_id, _) |
                        Event::OutgoingConnectionRequestAccepted(peer_id, _)) = event {
                                                assert_eq!(peer_id, *swarm3.local_peer_id());
                                                num_events_waiting -= 1;
                                            }
                                        }
                                        _ = swarm2.select_next_some() => {}
                                        event = swarm3.select_next_some() => {
                                            match event {
                                                SwarmEvent::Behaviour(Event::OutgoingConnectionRequestAccepted(peer_id, _) |
            Event::IncomingConnectionRequestAccepted(peer_id, _))=> {
                                                    assert_eq!(peer_id, *swarm1.local_peer_id());
                                                    num_events_waiting -= 1;
                                                }
                                                SwarmEvent::Behaviour(Event::IncomingConnectionRequestDiscarded(peer_id, _)) => {
                                                    assert_eq!(peer_id, *swarm2.local_peer_id());
                                                    num_events_waiting -= 1;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
        }
    }

    fn new_blend_swarm(
        keypair: Keypair,
        addr: Multiaddr,
        expected_duration: Duration,
        expected_message_range: Option<RangeInclusive<u64>>,
        membership_info: Option<Membership<PeerId>>,
        peering_degree: Option<RangeInclusive<usize>>,
    ) -> Swarm<Behaviour<TestTokioIntervalStreamProvider>> {
        new_swarm_with_behaviour(
            keypair,
            addr,
            Behaviour::new(
                &Config {
                    seen_message_cache_size: 1000,
                    peering_degree: peering_degree.unwrap_or(0..=1000),
                },
                TestTokioIntervalStreamProvider(
                    expected_duration,
                    // If no range is provided, we assume the maximum range which is equivalent
                    // to not having a monitor at all.
                    expected_message_range.unwrap_or(0..=u64::MAX),
                ),
                membership_info,
            ),
        )
    }

    fn new_dummy_swarm(keypair: Keypair, addr: Multiaddr) -> Swarm<dummy::Behaviour> {
        new_swarm_with_behaviour(keypair, addr, dummy::Behaviour)
    }

    fn new_swarm_with_behaviour<Behaviour: NetworkBehaviour>(
        keypair: Keypair,
        addr: Multiaddr,
        behaviour: Behaviour,
    ) -> Swarm<Behaviour> {
        let mut swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_other_transport(|keypair| {
                libp2p::quic::tokio::Transport::new(libp2p::quic::Config::new(keypair))
            })
            .unwrap()
            .with_behaviour(|_| behaviour)
            .unwrap()
            .with_swarm_config(|config| {
                // We want connections to be closed immediately as soon as
                // the corresponding streams are dropped by behaviours.
                config.with_idle_connection_timeout(Duration::ZERO)
            })
            .build();
        swarm.listen_on(addr).unwrap();
        swarm
    }

    fn nodes(
        count: usize,
        base_port: usize,
    ) -> (
        impl Iterator<Item = Node<PeerId>>,
        impl Iterator<Item = Keypair>,
    ) {
        let mut nodes = Vec::with_capacity(count);
        let mut keypairs = Vec::with_capacity(count);

        for i in 0..count {
            let keypair = Keypair::generate_ed25519();
            let node = Node {
                id: PeerId::from(keypair.public()),
                address: format!("/ip4/127.0.0.1/udp/{}/quic-v1", base_port + i)
                    .parse()
                    .unwrap(),
                public_key: Ed25519PrivateKey::from([i as u8; 32]).public_key(),
            };
            nodes.push(node);
            keypairs.push(keypair);
        }

        (nodes.into_iter(), keypairs.into_iter())
    }
}
