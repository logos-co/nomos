pub mod behaviour;

#[cfg(test)]
mod test {
    use std::time::Duration;

    use futures::StreamExt as _;
    use kzgrs::Proof;
    use kzgrs_backend::common::{share::DaLightShare, Column};
    use libp2p::{identity::Keypair, swarm::SwarmEvent, Multiaddr, PeerId, Swarm};
    use log::debug;
    use rand::Rng as _;
    use subnetworks_assignations::MembershipHandler;
    use tokio::time;
    use tokio_stream::wrappers::IntervalStream;
    use tracing_subscriber::{fmt::TestWriter, EnvFilter};

    use crate::{
        protocols::sampling::behaviour::{
            BehaviourSampleRes, SamplingBehaviour, SamplingEvent, SubnetsConfig,
        },
        test_utils::{new_swarm_in_memory, AllNeighbours},
        SubnetworkId,
    };

    async fn test_sampling_swarm(
        mut swarm: Swarm<
            SamplingBehaviour<
                impl MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static,
            >,
        >,
        msg_count: usize,
    ) -> Vec<[u8; 32]> {
        let mut res = vec![];
        loop {
            match swarm.next().await {
                None => {}
                Some(SwarmEvent::Behaviour(SamplingEvent::IncomingSample {
                    request_receiver,
                    response_sender,
                })) => {
                    debug!("Received request");
                    // spawn here because otherwise we block polling
                    tokio::spawn(request_receiver);
                    response_sender
                        .send(BehaviourSampleRes::SamplingSuccess {
                            blob_id: Default::default(),
                            subnetwork_id: Default::default(),
                            share: Box::new(DaLightShare {
                                column: Column(vec![]),
                                share_idx: 0,
                                combined_column_proof: Proof::default(),
                            }),
                        })
                        .unwrap();
                }
                Some(SwarmEvent::Behaviour(SamplingEvent::SamplingSuccess { blob_id, .. })) => {
                    debug!("Received response");
                    res.push(blob_id);
                }
                Some(SwarmEvent::Behaviour(SamplingEvent::SamplingError { error })) => {
                    debug!("Error during sampling: {error}");
                }
                Some(event) => {
                    debug!("{event:?}");
                }
            }
            if res.len() == msg_count {
                break res;
            }
        }
    }

    #[tokio::test]
    async fn test_sampling_two_peers() {
        const MSG_COUNT: usize = 10;

        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .compact()
            .with_writer(TestWriter::default())
            .try_init();
        let k1 = Keypair::generate_ed25519();
        let k2 = Keypair::generate_ed25519();

        // Generate a random peer ids not to conflict with other tests
        let p1_id = rand::thread_rng().gen::<u64>();
        let p1_address: Multiaddr = format!("/memory/{p1_id}").parse().unwrap();

        let p2_id = rand::thread_rng().gen::<u64>();
        let p2_address: Multiaddr = format!("/memory/{p2_id}").parse().unwrap();

        let neighbours_p1 = AllNeighbours::default();
        neighbours_p1.add_neighbour(PeerId::from_public_key(&k1.public()));
        neighbours_p1.add_neighbour(PeerId::from_public_key(&k2.public()));
        let p1_addresses = vec![(PeerId::from_public_key(&k2.public()), p2_address.clone())];
        neighbours_p1.update_addresses(p1_addresses);

        let neighbours_p2 = AllNeighbours::default();
        neighbours_p2.add_neighbour(PeerId::from_public_key(&k1.public()));
        neighbours_p2.add_neighbour(PeerId::from_public_key(&k2.public()));
        let p2_addresses = vec![(PeerId::from_public_key(&k1.public()), p1_address.clone())];
        neighbours_p2.update_addresses(p2_addresses);

        let p1_behavior = SamplingBehaviour::new(
            PeerId::from_public_key(&k1.public()),
            neighbours_p1.clone(),
            SubnetsConfig { num_of_subnets: 1 },
            Box::pin(IntervalStream::new(time::interval(Duration::from_secs(1))).map(|_| ())),
        );

        let mut p1 = new_swarm_in_memory(&k1, p1_behavior);

        let p2_behavior = SamplingBehaviour::new(
            PeerId::from_public_key(&k2.public()),
            neighbours_p1.clone(),
            SubnetsConfig { num_of_subnets: 1 },
            Box::pin(IntervalStream::new(time::interval(Duration::from_secs(1))).map(|_| ())),
        );
        let mut p2 = new_swarm_in_memory(&k2, p2_behavior);

        let request_sender_1 = p1.behaviour().sample_request_channel();
        let request_sender_2 = p2.behaviour().sample_request_channel();
        let _p1_address = p1_address.clone();
        let _p2_address = p2_address.clone();

        let t1 = tokio::spawn(async move {
            p1.listen_on(p1_address).unwrap();
            time::sleep(Duration::from_secs(1)).await;
            test_sampling_swarm(p1, MSG_COUNT).await
        });
        let t2 = tokio::spawn(async move {
            p2.listen_on(p2_address).unwrap();
            time::sleep(Duration::from_secs(1)).await;
            test_sampling_swarm(p2, MSG_COUNT).await
        });
        time::sleep(Duration::from_secs(2)).await;
        for i in 0..MSG_COUNT {
            request_sender_1.send([i as u8; 32]).unwrap();
            request_sender_2.send([i as u8; 32]).unwrap();
        }

        let res1 = t1.await.unwrap();
        let res2 = t2.await.unwrap();
        assert_eq!(res1, res2);
    }
}
