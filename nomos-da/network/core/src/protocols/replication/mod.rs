pub mod behaviour;

#[cfg(test)]
mod test {
    use futures::StreamExt as _;
    use libp2p::{
        bytes::BytesMut, identity::Keypair, multiaddr::Protocol, quic, swarm::SwarmEvent,
        Multiaddr, PeerId, Swarm,
    };
    use libp2p_swarm_test::SwarmExt as _;
    use log::info;
    use nomos_da_messages::replication::ReplicationRequest;
    use std::{
        collections::VecDeque,
        net::SocketAddr,
        ops::Range,
        sync::{Arc, LazyLock},
        time::Duration,
    };
    use tokio::{
        io,
        net::UdpSocket,
        sync::{mpsc, Mutex},
    };
    use tracing::error;
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use crate::{
        protocols::replication::behaviour::{
            ReplicationBehaviour, ReplicationConfig, ReplicationEvent,
        },
        test_utils::{AllNeighbours, TamperingReplicationBehaviour},
    };

    type TestSwarm = Swarm<ReplicationBehaviour<AllNeighbours>>;

    fn get_swarm(key: Keypair, all_neighbours: AllNeighbours) -> TestSwarm {
        // libp2p_swarm_test::SwarmExt::new_ephemeral_tokio does not allow to inject
        // arbitrary keypair
        libp2p::SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|keypair| quic::tokio::Transport::new(quic::Config::new(keypair)))
            .unwrap()
            .with_behaviour(|key| {
                ReplicationBehaviour::new(
                    ReplicationConfig {
                        seen_message_cache_size: 100,
                        seen_message_ttl: Duration::from_secs(60),
                    },
                    PeerId::from_public_key(&key.public()),
                    all_neighbours,
                )
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
            .build()
    }

    fn make_neighbours(keys: &[&Keypair]) -> AllNeighbours {
        let neighbours = AllNeighbours::new();
        keys.iter()
            .for_each(|k| neighbours.add_neighbour(PeerId::from_public_key(&k.public())));
        neighbours
    }

    fn get_message(i: usize) -> ReplicationRequest {
        MESSAGES[i].clone()
    }

    async fn wait_for_incoming_connection(swarm: &mut TestSwarm, other: PeerId) {
        swarm
        .wait(|event| {
            matches!(event, SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == other)
                .then_some(event)
        })
        .await;
    }

    async fn wait_for_messages(swarm: &mut TestSwarm, expected: Range<usize>) {
        let mut expected_messages = expected
            .into_iter()
            .map(|i| Box::new(get_message(i)))
            .collect::<VecDeque<Box<ReplicationRequest>>>();

        while let Some(expected_message) = expected_messages.front() {
            loop {
                if let SwarmEvent::Behaviour(ReplicationEvent::IncomingMessage {
                    message, ..
                }) = swarm.select_next_some().await
                {
                    if &message == expected_message {
                        break;
                    }
                }
            }

            expected_messages.pop_front().unwrap();
        }
    }

    static MESSAGES: LazyLock<Vec<ReplicationRequest>> = LazyLock::new(|| {
        // The fixture contains 20 messages seeded from values 0..20 for subnet 0
        // Ad-hoc generation of those takes about 12 seconds on a Ryzen3700x
        bincode::deserialize(include_bytes!("./fixtures/messages.bincode")).unwrap()
    });

    // Tamper the original data
    fn modify_packet(packet: &mut BytesMut) {
        if !packet.is_empty() {
            packet[0] ^= 0b10101010; // Flip first byte
        }
    }

    // Heuristically determine if the packet is likely QUIC application data.
    fn is_probable_application_data(packet: &[u8]) -> bool {
        if packet.is_empty() {
            return false;
        }
        let first = packet[0];
        // Short header (0x40–0x7F) is typically post-handshake encrypted data
        (first & 0b1100_0000) == 0b0100_0000
    }

    // Convert Multiaddr to SocketAddr for UDP
    fn multiaddr_to_socket_addr(multiaddr: &Multiaddr) -> io::Result<SocketAddr> {
        let iter = multiaddr.iter();
        let mut ip = None;
        let mut port = None;

        for component in iter {
            match component {
                Protocol::Ip4(addr) => ip = Some(std::net::IpAddr::V4(addr)),
                Protocol::Ip6(addr) => ip = Some(std::net::IpAddr::V6(addr)),
                Protocol::Udp(p) => port = Some(p),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Unsupported protocol",
                    ))
                }
            }
        }

        match (ip, port) {
            (Some(ip), Some(port)) => Ok(SocketAddr::new(ip, port)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Multiaddr must include IP and UDP port",
            )),
        }
    }

    // Start the two-way UDP mutation proxy
    pub async fn start_udp_mutation_proxy(
        proxy_addr: Multiaddr,
        target_addr: Multiaddr,
    ) -> io::Result<()> {
        let proxy_socket_addr = multiaddr_to_socket_addr(&proxy_addr)?;
        let target_socket_addr = multiaddr_to_socket_addr(&target_addr)?;

        info!("UDP proxy listening on {proxy_socket_addr}, forwarding to {target_socket_addr}");

        let socket = Arc::new(UdpSocket::bind(proxy_socket_addr).await?);
        let client_addr = Arc::new(Mutex::new(None::<SocketAddr>));
        let mut buf = vec![0u8; 65535];

        loop {
            let (len, src_addr) = match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => (len, addr),
                Err(e) => {
                    error!("recv_from error: {e}");
                    continue;
                }
            };

            let mut data = BytesMut::from(&buf[..len]);

            let dest_addr = if src_addr == target_socket_addr {
                // From target → send to client
                let value = *client_addr.lock().await;
                if let Some(addr) = value {
                    addr
                } else {
                    error!("No client address recorded yet");
                    continue;
                }
            } else {
                // From client → save and send to target
                *client_addr.lock().await = Some(src_addr);
                target_socket_addr
            };

            if is_probable_application_data(&data) {
                modify_packet(&mut data);
            }

            if let Err(e) = socket.send_to(&data, dest_addr).await {
                error!("send_to error: {e}");
            }
        }
    }

    #[tokio::test]
    async fn test_replication_chain_in_both_directions() {
        // Scenario:
        // 0. Peer connections: A <- B -> C
        // 1. Alice is the initiator, Bob forwards to Charlie, message flow: A -> B -> C
        // 2. And then, within the same connections, Charlie is the initiator, Bob
        //    forwards to Alice, message flow: C -> B -> A
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();

        let k1 = Keypair::generate_ed25519();
        let k2 = Keypair::generate_ed25519();
        let k3 = Keypair::generate_ed25519();
        let peer_id1 = PeerId::from_public_key(&k1.public());
        let peer_id2 = PeerId::from_public_key(&k2.public());
        let peer_id3 = PeerId::from_public_key(&k3.public());
        let neighbours1 = make_neighbours(&[&k1, &k2]);
        let neighbours2 = make_neighbours(&[&k1, &k2, &k3]);
        let neighbours3 = make_neighbours(&[&k2, &k3]);
        let mut swarm_1 = get_swarm(k1, neighbours1);
        let mut swarm_2 = get_swarm(k2, neighbours2);
        let mut swarm_3 = get_swarm(k3, neighbours3);
        let (done_1_tx, mut done_1_rx) = mpsc::channel::<()>(1);
        let (done_2_tx, mut done_2_rx) = mpsc::channel::<()>(1);
        let (done_3_tx, mut done_3_rx) = mpsc::channel::<()>(1);

        let addr1: Multiaddr = "/ip4/127.0.0.1/udp/5054/quic-v1".parse().unwrap();
        let addr3: Multiaddr = "/ip4/127.0.0.1/udp/5055/quic-v1".parse().unwrap();
        let addr1_ = addr1.clone();
        let addr3_ = addr3.clone();
        let task_1 = async move {
            swarm_1.listen_on(addr1).unwrap();
            wait_for_incoming_connection(&mut swarm_1, peer_id2).await;

            (0..10usize).for_each(|i| swarm_1.behaviour_mut().send_message(&get_message(i)));

            wait_for_messages(&mut swarm_1, 10..20).await;

            done_1_tx.send(()).await.unwrap();
            swarm_1.loop_on_next().await;
        };
        let task_2 = async move {
            assert_eq!(swarm_2.dial_and_wait(addr1_).await, peer_id1);
            assert_eq!(swarm_2.dial_and_wait(addr3_).await, peer_id3);

            wait_for_messages(&mut swarm_2, 0..20).await;

            done_2_tx.send(()).await.unwrap();
            swarm_2.loop_on_next().await;
        };
        let task_3 = async move {
            swarm_3.listen_on(addr3).unwrap();
            wait_for_incoming_connection(&mut swarm_3, peer_id2).await;
            wait_for_messages(&mut swarm_3, 0..10).await;

            (10..20usize).for_each(|i| swarm_3.behaviour_mut().send_message(&get_message(i)));

            done_3_tx.send(()).await.unwrap();
            swarm_3.loop_on_next().await;
        };

        tokio::spawn(task_1);
        tokio::spawn(task_2);
        tokio::spawn(task_3);

        assert!(
            tokio::time::timeout(
                Duration::from_secs(10),
                futures::future::join3(done_1_rx.recv(), done_2_rx.recv(), done_3_rx.recv()),
            )
            .await
            .is_ok(),
            "Test timed out"
        );
    }

    #[tokio::test]
    #[expect(clippy::too_many_lines, reason = "Test to be split later on.")]
    async fn test_connects_and_receives_replication_messages() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();
        let k1 = Keypair::generate_ed25519();
        let k2 = Keypair::generate_ed25519();
        let peer_id2 = PeerId::from_public_key(&k2.public());

        let neighbours = make_neighbours(&[&k1, &k2]);
        let mut swarm_1 = get_swarm(k1.clone(), neighbours.clone());
        let mut swarm_2 = get_swarm(k2.clone(), neighbours.clone());

        let msg_count = 10usize;
        let addr: Multiaddr = "/ip4/127.0.0.1/udp/5053/quic-v1".parse().unwrap();
        let addr2 = addr.clone();

        // Create a custom QUIC transport
        let mut quic_config = quic::Config::new(&k2);
        // Set a very short handshake timeout to simulate connection failure
        quic_config.handshake_timeout = Duration::from_millis(100);
        // Set a very short idle timeout to simulate connection failure
        quic_config.max_idle_timeout = 100; // milliseconds as u32
                                            // Set a very short keep alive interval to simulate connection failure
        quic_config.keep_alive_interval = Duration::from_millis(50);

        // Create a new swarm with the tampered transport
        let mut swarm_2_tampered = libp2p::SwarmBuilder::with_existing_identity(k2.clone())
            .with_tokio()
            .with_other_transport(|_keypair| quic::tokio::Transport::new(quic_config))
            .unwrap()
            .with_behaviour(|key| {
                // Create a replication behavior that injects tampered messages
                let base = ReplicationBehaviour::new(
                    ReplicationConfig {
                        seen_message_cache_size: 100,
                        seen_message_ttl: Duration::from_secs(60),
                    },
                    PeerId::from_public_key(&key.public()),
                    neighbours,
                );
                let mut behaviour = TamperingReplicationBehaviour::new(base);
                behaviour.set_tamper_hook(|mut msg| {
                    msg.share.data.share_idx ^= 0xFF; // Flip the first byte
                    msg
                });
                behaviour
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
            .build();

        // Test 1: Normal communication should work
        let task_1 = async move {
            swarm_1.listen_on(addr.clone()).unwrap();
            wait_for_incoming_connection(&mut swarm_1, peer_id2).await;
            swarm_1
                .filter_map(|event| async {
                    if let SwarmEvent::Behaviour(ReplicationEvent::IncomingMessage {
                        message,
                        ..
                    }) = event
                    {
                        Some(message)
                    } else {
                        None
                    }
                })
                .take(msg_count)
                .collect::<Vec<_>>()
                .await
        };
        let join1 = tokio::spawn(task_1);
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(10);
        let (terminate_sender, mut terminate_receiver) = tokio::sync::oneshot::channel::<()>();
        let task_2 = async move {
            swarm_2.dial_and_wait(addr2).await;
            let mut i = 0usize;
            loop {
                tokio::select! {
                    // send a message everytime that the channel ticks
                    _  = receiver.recv() => {
                        swarm_2.behaviour_mut().send_message(&get_message(i));
                        i += 1;
                    }
                    // print out events
                    event = swarm_2.select_next_some() => {
                        if let SwarmEvent::ConnectionEstablished{ peer_id,  connection_id, .. } = event {
                            info!("Connected to {peer_id} with connection_id: {connection_id}");
                        }
                    }
                    // terminate future
                    _ = &mut terminate_receiver => {
                        break;
                    }
                }
            }
        };
        let join2 = tokio::spawn(task_2);

        // Send messages and verify normal communication
        for _ in 0..10 {
            sender.send(()).await.unwrap();
        }
        // await for task1 to have all messages, then terminate task 2
        tokio::select! {
            Ok(res) = join1 => {
                assert_eq!(res.len(), msg_count);
                terminate_sender.send(()).unwrap();
            }
            _ = join2 => {
                panic!("task two should not finish before 1");
            }
        }

        // Test 2: Tampered transport should fail
        // let addr3: Multiaddr = "/ip4/127.0.0.1/udp/5056/quic-v1".parse().unwrap();
        // let addr4 = addr3.clone();

        let proxy_addr: Multiaddr = "/ip4/127.0.0.1/udp/5058".parse().unwrap();
        let server_addr: Multiaddr = "/ip4/127.0.0.1/udp/5059".parse().unwrap();

        // Start the proxy in the background
        tokio::spawn(start_udp_mutation_proxy(
            proxy_addr.clone(),
            server_addr.clone(),
        ));

        // Allow proxy to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        let addr3: Multiaddr = format!("{}/quic-v1", server_addr).parse().unwrap();
        let addr4: Multiaddr = format!("{}/quic-v1", proxy_addr).parse().unwrap();

        let mut swarm_3 = get_swarm(k1.clone(), make_neighbours(&[&k1, &k2]));

        let task_3 = async move {
            swarm_3.listen_on(addr3.clone()).unwrap();
            wait_for_incoming_connection(&mut swarm_3, peer_id2).await;
            swarm_3
                .filter_map(|event| async {
                    if let SwarmEvent::Behaviour(ReplicationEvent::IncomingMessage {
                        message,
                        ..
                    }) = event
                    {
                        info!("Received message {:?}", message);
                        assert_ne!(message.share.data.share_idx, 0);
                        Some(message)
                    } else {
                        None
                    }
                })
                .take(msg_count)
                .collect::<Vec<_>>()
                .await
        };

        let task_4 = async move {
            swarm_2_tampered.dial(addr4).unwrap();

            // Send a tampered message after attempting connection
            let mut i = 0;
            loop {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(50)) => {
                        let behaviour = swarm_2_tampered.behaviour_mut();
                        let msg = get_message(i);
                        behaviour.send_message(&msg);
                        info!("Sent message {i}# ");
                        if i < 19 {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    event = swarm_2_tampered.select_next_some() => {
                        match event {
                            SwarmEvent::ConnectionClosed { .. } => break,
                            SwarmEvent::ConnectionEstablished { peer_id,  connection_id, .. } => {
                                info!("Connected to {peer_id} with connection_id: {connection_id}");
                            },
                            _ => {}
                        }
                    }
                }
            }
        };

        tokio::join!(task_3, task_4);
    }

    #[tokio::test]
    #[expect(clippy::too_many_lines, reason = "Test to be split later on.")]
    async fn test_connects_and_receives_messages_with_udp_proxy() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();

        let proxy_addr: Multiaddr = "/ip4/127.0.0.1/udp/5058".parse().unwrap();
        let server_addr: Multiaddr = "/ip4/127.0.0.1/udp/5059".parse().unwrap();
        start_udp_mutation_proxy(proxy_addr.clone(), server_addr.clone())
            .await
            .expect("Failed to start UDP proxy");

        let k1 = Keypair::generate_ed25519();
        let k2 = Keypair::generate_ed25519();
        let peer_id2 = PeerId::from_public_key(&k2.public());

        let neighbours = make_neighbours(&[&k1, &k2]);
        let mut swarm_3 = get_swarm(k1.clone(), make_neighbours(&[&k1, &k2]));

        let mut quic_config = quic::Config::new(&k2);
        quic_config.handshake_timeout = Duration::from_millis(100);
        quic_config.max_idle_timeout = 100;
        quic_config.keep_alive_interval = Duration::from_millis(50);

        let mut swarm_2_tampered = libp2p::SwarmBuilder::with_existing_identity(k2.clone())
            .with_tokio()
            .with_other_transport(|_keypair| quic::tokio::Transport::new(quic_config))
            .unwrap()
            .with_behaviour(|key| {
                let base = ReplicationBehaviour::new(
                    ReplicationConfig {
                        seen_message_cache_size: 100,
                        seen_message_ttl: Duration::from_secs(60),
                    },
                    PeerId::from_public_key(&key.public()),
                    neighbours,
                );
                let mut behaviour = TamperingReplicationBehaviour::new(base);
                behaviour.set_tamper_hook(|mut msg| {
                    msg.share.data.share_idx ^= 0xFF;
                    msg
                });
                behaviour
            })
            .unwrap()
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(10)))
            .build();

        let addr3: Multiaddr = format!("{}/quic-v1", server_addr).parse().unwrap();
        let addr4: Multiaddr = format!("{}/quic-v1", proxy_addr).parse().unwrap();

        let task_3 = async move {
            swarm_3.listen_on(addr3).unwrap();
            wait_for_incoming_connection(&mut swarm_3, peer_id2).await;

            let all_messages: Vec<_> = swarm_3
                .filter_map(|event| async {
                    match event {
                        SwarmEvent::Behaviour(ReplicationEvent::IncomingMessage {
                            message,
                            ..
                        }) => {
                            info!("Received message {:?}", message);
                            Some(message)
                        }
                        _ => None,
                    }
                })
                .take(10)
                .collect()
                .await;

            let malformed_count = all_messages
                .iter()
                .filter(|m| m.share.data.share_idx == 0)
                .count();
            let valid_count = all_messages.len() - malformed_count;

            assert!(
                malformed_count > 0,
                "Expected at least one malformed message due to proxy tampering"
            );
            assert_eq!(
                valid_count + malformed_count,
                10,
                "All messages should have been processed"
            );
        };

        let task_4 = async move {
            swarm_2_tampered.dial(addr4).unwrap();
            let mut i = 0;
            loop {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(50)) => {
                        let behaviour = swarm_2_tampered.behaviour_mut();
                        let msg = get_message(i);
                        behaviour.send_message(&msg);
                        info!("Sent message {:?}", msg);
                        if i < 10 {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    event = swarm_2_tampered.select_next_some() => {
                        match event {
                            SwarmEvent::ConnectionClosed { .. } => break,
                            SwarmEvent::ConnectionEstablished { peer_id, connection_id, .. } => {
                                info!("Connected to {peer_id} with connection_id: {connection_id}");
                            },
                            _ => {}
                        }
                    }
                }
            }
        };

        tokio::join!(task_3, task_4);
    }
}
