pub mod behaviour;
pub mod handler;

#[cfg(test)]
mod test {
    use crate::protocols::replication::behaviour::{ReplicationBehaviour, ReplicationEvent};
    use crate::protocols::replication::handler::DaMessage;
    use crate::test_utils::AllNeighbours;
    use futures::StreamExt;
    use kzgrs_backend::common::blob::DaBlob;
    use kzgrs_backend::encoder;
    use kzgrs_backend::encoder::DaEncoderParams;
    use libp2p::identity::Keypair;
    use libp2p::swarm::SwarmEvent;
    use libp2p::{quic, Multiaddr, PeerId, Swarm};
    use log::info;
    use nomos_core::da::{BlobId, DaEncoder};
    use nomos_da_messages::common::Blob;
    use std::time::Duration;
    use tracing_subscriber::fmt::TestWriter;
    use tracing_subscriber::EnvFilter;

    fn get_encoder() -> encoder::DaEncoder {
        const DOMAIN_SIZE: usize = 16;
        let params = DaEncoderParams::default_with(DOMAIN_SIZE);
        encoder::DaEncoder::new(params)
    }

    fn get_da_blob(data: Option<Vec<u8>>) -> DaBlob {
        let encoder = get_encoder();

        let data = data.unwrap_or_else(|| {
            vec![
                49u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
            ]
        });

        let encoded_data = encoder.encode(&data).unwrap();
        let columns: Vec<_> = encoded_data.extended_data.columns().collect();

        let index = 0;
        let da_blob = DaBlob {
            column: columns[index].clone(),
            column_idx: index
                .try_into()
                .expect("Column index shouldn't overflow the target type"),
            column_commitment: encoded_data.column_commitments[index],
            aggregated_column_commitment: encoded_data.aggregated_column_commitment,
            aggregated_column_proof: encoded_data.aggregated_column_proofs[index],
            rows_commitments: encoded_data.row_commitments.clone(),
            rows_proofs: encoded_data
                .rows_proofs
                .iter()
                .map(|proofs| proofs.get(index).cloned().unwrap())
                .collect(),
        };

        da_blob
    }

    #[tokio::test]
    async fn test_connects_and_receives_replication_messages() {
        fn get_swarm(
            key: Keypair,
            all_neighbours: AllNeighbours,
        ) -> Swarm<ReplicationBehaviour<AllNeighbours>> {
            libp2p::SwarmBuilder::with_existing_identity(key)
                .with_tokio()
                .with_other_transport(|keypair| {
                    quic::tokio::Transport::new(quic::Config::new(keypair))
                })
                .unwrap()
                .with_behaviour(|key| {
                    ReplicationBehaviour::new(
                        PeerId::from_public_key(&key.public()),
                        all_neighbours,
                    )
                })
                .unwrap()
                .with_swarm_config(|cfg| {
                    cfg.with_idle_connection_timeout(std::time::Duration::from_secs(u64::MAX))
                })
                .build()
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();
        let k1 = libp2p::identity::Keypair::generate_ed25519();
        let k2 = libp2p::identity::Keypair::generate_ed25519();

        let neighbours = AllNeighbours {
            neighbours: [
                PeerId::from_public_key(&k1.public()),
                PeerId::from_public_key(&k2.public()),
            ]
            .into_iter()
            .collect(),
        };
        let mut swarm_1 = get_swarm(k1, neighbours.clone());
        let mut swarm_2 = get_swarm(k2, neighbours);

        let msg_count = 10usize;
        let addr: Multiaddr = "/ip4/127.0.0.1/udp/5053/quic-v1".parse().unwrap();
        let addr2 = addr.clone();
        // future that listens for messages and collects `msg_count` of them, then returns them
        let task_1 = async move {
            swarm_1.listen_on(addr.clone()).unwrap();
            let res = swarm_1
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
                .await;
            res
        };
        let join1 = tokio::spawn(task_1);
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(10);
        let (terminate_sender, mut terminate_receiver) = tokio::sync::oneshot::channel::<()>();
        let task_2 = async move {
            swarm_2.dial(addr2).unwrap();
            let mut i = 0usize;
            loop {
                tokio::select! {
                    // send a message everytime that the channel ticks
                    _  = receiver.recv() => {
                        let blob_id_bytes: [u8; 32] = i.to_be_bytes().to_vec().try_into().unwrap();

                        let blob = Blob::new(
                            BlobId::from(blob_id_bytes),
                            get_da_blob(None)
                        );
                        swarm_2.behaviour_mut().send_message(DaMessage::new(blob, 0));
                        i += 1;
                    }
                    // print out events
                    event = swarm_2.select_next_some() => {
                        match event {
                            SwarmEvent::ConnectionEstablished{ peer_id,  connection_id, .. } => {
                                info!("Connected to {peer_id} with connection_id: {connection_id}");
                            }
                            _ => {}
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
        tokio::time::sleep(Duration::from_secs(1)).await;
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
}
