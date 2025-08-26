pub mod behaviour;

#[cfg(test)]
mod test {
    use std::{
        collections::{HashSet, VecDeque},
        ops::Range,
        path::PathBuf,
        sync::LazyLock,
        time::Duration,
    };

    use futures::StreamExt as _;
    use kzgrs_backend::testutils;
    use libp2p::{
        identity::{Keypair, PublicKey},
        quic,
        swarm::SwarmEvent,
        Multiaddr, PeerId, Swarm,
    };
    use libp2p_swarm_test::SwarmExt as _;
    use log::info;
    use nomos_core::{
        mantle::{
            ledger::Tx as LedgerTx,
            ops::{
                channel::{blob::BlobOp, ChannelId, Ed25519PublicKey, MsgId},
                Op,
            },
            MantleTx, SignedMantleTx, Transaction as _,
        },
        proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
    };
    use nomos_da_messages::{
        common::Share,
        replication::{ReplicationRequest, ReplicationResponseId},
    };
    use tokio::sync::mpsc;
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use crate::{
        protocols::replication::behaviour::{
            ReplicationBehaviour, ReplicationConfig, ReplicationEvent,
        },
        test_utils::AllNeighbours,
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
        for k in keys {
            neighbours.add_neighbour(PeerId::from_public_key(&k.public()));
        }
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
                    if *message == *expected_message.as_ref() {
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

    #[test]
    #[ignore = "Invoke to generate fixtures"]
    fn generate_fixtures() {
        let path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("src/protocols/replication/fixtures/messages.bincode");
        let mut blob_id = [0u8; 32];

        let messages = (0..20u8)
            .map(|i| {
                blob_id[31] = i;
                ReplicationRequest::from(Share {
                    blob_id,
                    data: {
                        let mut data = testutils::get_default_da_blob_data();
                        *data.last_mut().unwrap() = i;
                        testutils::get_da_share(Some(data))
                    },
                })
            })
            .collect::<Vec<_>>();
        let serialized = bincode::serialize(&messages).unwrap();
        std::fs::write(path, serialized).unwrap();
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
        let mut swarm_1 = get_swarm(k1, neighbours.clone());
        let mut swarm_2 = get_swarm(k2, neighbours);

        let msg_count = 10usize;
        let addr: Multiaddr = "/ip4/127.0.0.1/udp/5053/quic-v1".parse().unwrap();
        let addr2 = addr.clone();
        // future that listens for messages and collects `msg_count` of them, then
        // returns them
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
        let (sender, mut receiver) = mpsc::channel::<()>(10);
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
        // send 10 messages
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
    }

    fn get_ed25519_bytes(pubkey: PublicKey) -> Option<[u8; 32]> {
        pubkey.try_into_ed25519().ok().map(|pk| pk.to_bytes())
    }

    #[tokio::test]
    async fn test_tx_replication() {
        const TX_COUNT: u64 = 5;
        let k1 = Keypair::generate_ed25519();
        let k2 = Keypair::generate_ed25519();
        let peer_id2 = PeerId::from_public_key(&k2.public());
        let signer_bytes_k2 = get_ed25519_bytes(k1.public()).expect("Public key must be Ed25519");

        let neighbours = make_neighbours(&[&k1, &k2]);

        let mut swarm1 = get_swarm(k1, neighbours.clone());
        let mut swarm2 = get_swarm(k2, neighbours);

        let base_op = Op::ChannelBlob(BlobOp {
            channel: ChannelId::from([2; 32]),
            blob: [0u8; 32],
            blob_size: 0,
            da_storage_gas_price: 0,
            parent: MsgId::root(),
            signer: Ed25519PublicKey::from_bytes(&signer_bytes_k2).unwrap(),
        });

        let base_mantle_tx = MantleTx {
            ops: vec![base_op],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            storage_gas_price: 0,
            execution_gas_price: 0,
        };

        let base_signed_tx = SignedMantleTx {
            ops_profs: Vec::new(),
            ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
                msg_hash: base_mantle_tx.hash().into(),
                pks: vec![],
            }),
            mantle_tx: base_mantle_tx,
        };

        let addr1: Multiaddr = "/ip4/127.0.0.1/udp/5056/quic-v1".parse().unwrap();
        swarm1.listen_on(addr1.clone()).unwrap();

        let task1 = async move {
            swarm2.dial_and_wait(addr1).await;

            for i in 0..TX_COUNT {
                let mut unique_signed_tx = base_signed_tx.clone();
                // Mantle op payload is not yet included in the signed bytes for tx hash, but
                // storage_gas_price also affect the hash and is enough in this test case.
                unique_signed_tx.mantle_tx.storage_gas_price = i;

                let tx_message = ReplicationRequest::from(unique_signed_tx.clone());

                // Send each message two times.
                swarm2.behaviour_mut().send_message(&tx_message);
                swarm2.behaviour_mut().send_message(&tx_message);
            }

            swarm2.loop_on_next().await;
        };

        tokio::spawn(task1);

        wait_for_incoming_connection(&mut swarm1, peer_id2).await;

        let mut received_tx_ids: HashSet<ReplicationResponseId> = HashSet::new();
        let mut duplicate_messages_count = 0;

        for _ in 0..TX_COUNT {
            let event = tokio::time::timeout(
                Duration::from_secs(5),
                swarm1.wait(|event| {
                    if let SwarmEvent::Behaviour(ReplicationEvent::IncomingMessage {
                        message,
                        ..
                    }) = event
                    {
                        if let ReplicationRequest::Tx(_) = message.as_ref() {
                            return Some(message);
                        }
                    }
                    None
                }),
            )
            .await
            .expect("Swarm1 should receive all Tx messages within timeout");

            let received_tx_id = event.id();
            if !received_tx_ids.insert(received_tx_id) {
                duplicate_messages_count += 1;
            }
        }

        assert_eq!(received_tx_ids.len(), TX_COUNT as usize, "Txs not received");
        assert_eq!(
            duplicate_messages_count, 0,
            "No duplicate Tx messages should be replicated."
        );
    }
}
