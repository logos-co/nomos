pub mod behaviour;
mod connections;
pub mod errors;
mod streams;

use errors::SamplingError;
use futures::{
    channel::oneshot::{Receiver, Sender},
    future::BoxFuture,
};
use kzgrs_backend::common::{
    share::{DaLightShare, DaSharesCommitments},
    ShareIndex,
};
use libp2p::PeerId;
use nomos_core::da::BlobId;
use nomos_da_messages::{common, sampling};
use serde::{Deserialize, Serialize};
use streams::SampleStream;

use crate::SubnetworkId;

enum SampleStreamResponse {
    Writer(Box<sampling::SampleResponse>),
    Reader,
}

type SampleFutureSuccess = (PeerId, SampleStreamResponse, SampleStream);
type SampleFutureError = (SamplingError, Option<SampleStream>);

type SamplingStreamFuture = BoxFuture<'static, Result<SampleFutureSuccess, SampleFutureError>>;

/// Auxiliary struct that binds where to send a request and the pair channel to
/// listen for a response
struct ResponseChannel {
    request_sender: Sender<BehaviourSampleReq>,
    response_receiver: Receiver<BehaviourSampleRes>,
}

#[derive(Debug, Clone)]
pub enum BehaviourSampleReq {
    Share {
        blob_id: BlobId,
        share_idx: ShareIndex,
    },
    Commitments {
        blob_id: BlobId,
    },
}

impl TryFrom<sampling::SampleRequest> for BehaviourSampleReq {
    type Error = Vec<u8>;

    fn try_from(req: sampling::SampleRequest) -> Result<Self, Self::Error> {
        match req {
            sampling::SampleRequest::Share(sampling::SampleShare { blob_id, share_idx }) => {
                Ok(Self::Share { blob_id, share_idx })
            }
            sampling::SampleRequest::Commitments(sampling::SampleCommitments { blob_id }) => {
                Ok(Self::Commitments { blob_id })
            }
        }
    }
}

#[derive(Debug)]
pub enum BehaviourSampleRes {
    SamplingSuccess {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        share: Box<DaLightShare>,
    },
    CommitmentsSuccess {
        blob_id: BlobId,
        commitments: Box<DaSharesCommitments>,
    },
    SampleNotFound {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
    },
    CommitmentsNotFound {
        blob_id: BlobId,
    },
}

impl From<BehaviourSampleRes> for sampling::SampleResponse {
    fn from(res: BehaviourSampleRes) -> Self {
        match res {
            BehaviourSampleRes::SamplingSuccess { share, blob_id, .. } => {
                Self::Share(common::LightShare::new(blob_id, *share))
            }
            BehaviourSampleRes::CommitmentsSuccess { commitments, .. } => {
                Self::Commitments(*commitments)
            }
            BehaviourSampleRes::SampleNotFound {
                blob_id,
                subnetwork_id,
            } => Self::Error(sampling::SampleError::new_share(
                blob_id,
                subnetwork_id,
                sampling::SampleErrorType::NotFound,
                "Sample not found",
            )),
            BehaviourSampleRes::CommitmentsNotFound { blob_id } => {
                Self::Error(sampling::SampleError::new_commitments(
                    blob_id,
                    sampling::SampleErrorType::NotFound,
                    "Commitments not found",
                ))
            }
        }
    }
}

#[derive(Debug)]
pub enum SamplingEvent {
    /// A blob successfully arrived its destination
    SamplingSuccess {
        blob_id: BlobId,
        subnetwork_id: SubnetworkId,
        light_share: Box<DaLightShare>,
    },
    CommitmentsSuccess {
        blob_id: BlobId,
        commitments: Box<DaSharesCommitments>,
    },
    IncomingSample {
        request_receiver: Receiver<BehaviourSampleReq>,
        response_sender: Sender<BehaviourSampleRes>,
    },
    SamplingError {
        error: SamplingError,
    },
}

impl SamplingEvent {
    #[must_use]
    pub const fn no_subnetwork_peers_err(blob_id: BlobId, subnetwork_id: SubnetworkId) -> Self {
        Self::SamplingError {
            error: SamplingError::NoSubnetworkPeers {
                blob_id,
                subnetwork_id,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubnetsConfig {
    /// Number of unique subnets that samples should be taken from when sampling
    /// a blob.
    pub num_of_subnets: usize,
    /// Numer of connection attemps to the peers in a subnetwork, if previous
    /// connection attempt failed.
    pub shares_retry_limit: usize,
    /// Number of attemts for retrieving commitments from a random sampling
    /// peers that are selected for connections.
    pub commitments_retry_limit: usize,
}

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

    use super::{behaviour::SamplingBehaviour, SamplingEvent};
    use crate::{
        addressbook::AddressBookHandler,
        protocols::sampling::{BehaviourSampleRes, SubnetsConfig},
        test_utils::{new_swarm_in_memory, AllNeighbours},
        SubnetworkId,
    };

    async fn test_sampling_swarm(
        mut swarm: Swarm<
            SamplingBehaviour<
                impl MembershipHandler<Id = PeerId, NetworkId = SubnetworkId> + 'static,
                impl AddressBookHandler<Id = PeerId> + 'static,
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
            neighbours_p1.clone(),
            SubnetsConfig {
                num_of_subnets: 1,
                shares_retry_limit: 1,
                commitments_retry_limit: 1,
            },
            Box::pin(IntervalStream::new(time::interval(Duration::from_secs(1))).map(|_| ())),
        );

        let mut p1 = new_swarm_in_memory(&k1, p1_behavior);

        let p2_behavior = SamplingBehaviour::new(
            PeerId::from_public_key(&k2.public()),
            neighbours_p2.clone(),
            neighbours_p2.clone(),
            SubnetsConfig {
                num_of_subnets: 1,
                shares_retry_limit: 1,
                commitments_retry_limit: 1,
            },
            Box::pin(IntervalStream::new(time::interval(Duration::from_secs(1))).map(|_| ())),
        );
        let mut p2 = new_swarm_in_memory(&k2, p2_behavior);

        let request_sender_1 = p1.behaviour().shares_request_channel();
        let request_sender_2 = p2.behaviour().shares_request_channel();
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
