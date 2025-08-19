use std::{collections::HashSet, fmt::Debug, time::Duration};

use futures::{
    channel::oneshot::{Receiver, Sender},
    StreamExt,
};
use kzgrs_backend::common::{
    share::{DaLightShare, DaShare, DaSharesCommitments},
    ShareIndex,
};
use nomos_core::{block::BlockNumber, da::BlobId};
use nomos_da_network_core::{
    maintenance::{balancer::ConnectionBalancerCommand, monitor::ConnectionMonitorCommand},
    protocols::sampling::{
        self, errors::SamplingError, BehaviourSampleReq, BehaviourSampleRes, SubnetsConfig,
    },
    swarm::{
        validator::{SampleArgs, ValidatorEventsStream},
        DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
    },
};
use nomos_libp2p::{ed25519, secret_key_serde, Multiaddr};
use serde::{Deserialize, Serialize};
use tokio::sync::{
    broadcast, mpsc,
    mpsc::{error::SendError, UnboundedSender},
    oneshot,
};
use tracing::error;

pub(crate) const BROADCAST_CHANNEL_SIZE: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaNetworkBackendSettings {
    // Identification Secp256k1 private key in Hex format (`0x123...abc`). Default random.
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    pub listening_address: Multiaddr,
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
    pub replication_settings: ReplicationConfig,
    pub subnets_settings: SubnetsConfig,
    pub refresh_interval: Duration,
}

/// Sampling events coming from da network
#[derive(Debug, Clone)]
pub enum SamplingEvent {
    /// A success sampling
    SamplingSuccess {
        blob_id: BlobId,
        light_share: Box<DaLightShare>,
    },
    /// Incoming sampling request
    SamplingRequest {
        blob_id: BlobId,
        share_idx: ShareIndex,
        response_sender: mpsc::Sender<Option<DaLightShare>>,
    },
    /// A failed sampling error
    SamplingError { error: SamplingError },
}

/// Commitments events coming from da network.
#[derive(Debug, Clone)]
pub enum CommitmentsEvent {
    /// A success sampling
    CommitmentsSuccess {
        blob_id: BlobId,
        commitments: Box<DaSharesCommitments>,
    },
    /// Incoming sampling request
    CommitmentsRequest {
        blob_id: BlobId,
        response_sender: mpsc::Sender<Option<DaSharesCommitments>>,
    },
    /// A failed sampling error
    CommitmentsError { error: SamplingError },
}

impl SamplingEvent {
    #[must_use]
    pub fn blob_id(&self) -> Option<&BlobId> {
        match self {
            Self::SamplingRequest { blob_id, .. } | Self::SamplingSuccess { blob_id, .. } => {
                Some(blob_id)
            }
            Self::SamplingError { error } => error.blob_id(),
        }
    }

    #[must_use]
    pub fn has_blob_id(&self, target: &BlobId) -> bool {
        self.blob_id() == Some(target)
    }
}

/// Task that handles forwarding of events to the subscriptions channels/stream
pub(crate) async fn handle_validator_events_stream(
    events_streams: ValidatorEventsStream,
    sampling_broadcast_sender: broadcast::Sender<SamplingEvent>,
    commitments_broadcast_sender: broadcast::Sender<CommitmentsEvent>,
    validation_broadcast_sender: broadcast::Sender<DaShare>,
) {
    let ValidatorEventsStream {
        mut sampling_events_receiver,
        mut validation_events_receiver,
    } = events_streams;
    loop {
        // WARNING: `StreamExt::next` is cancellation safe.
        // If adding more branches check if such methods are within the cancellation
        // safe set: https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety
        tokio::select! {
            Some(sampling_event) = StreamExt::next(&mut sampling_events_receiver) => {
                handle_event(&sampling_broadcast_sender, &commitments_broadcast_sender, sampling_event).await;
            }
            Some(da_share) = StreamExt::next(&mut validation_events_receiver) => {
                if let Err(error) = validation_broadcast_sender.send(da_share) {
                    error!("Error in internal broadcast of validation for blob: {:?}", error.0);
                }
            }
        }
    }
}

async fn handle_event(
    sampling_broadcast_sender: &broadcast::Sender<SamplingEvent>,
    commitments_broadcast_sender: &broadcast::Sender<CommitmentsEvent>,
    sampling_event: sampling::SamplingEvent,
) {
    match sampling_event {
        sampling::SamplingEvent::SamplingSuccess {
            blob_id,
            light_share,
            ..
        } => {
            if let Err(e) = sampling_broadcast_sender.send(SamplingEvent::SamplingSuccess {
                blob_id,
                light_share,
            }) {
                error!("Error in internal broadcast of sampling success: {e:?}");
            }
        }
        sampling::SamplingEvent::CommitmentsSuccess {
            blob_id,
            commitments,
        } => {
            if let Err(e) =
                commitments_broadcast_sender.send(CommitmentsEvent::CommitmentsSuccess {
                    blob_id,
                    commitments,
                })
            {
                error!("Error in internal broadcast of sampling success: {e:?}");
            }
        }
        sampling::SamplingEvent::IncomingSample {
            request_receiver,
            response_sender,
        } => {
            handle_request(
                sampling_broadcast_sender,
                commitments_broadcast_sender,
                request_receiver,
                response_sender,
            )
            .await;
        }
        sampling::SamplingEvent::SamplingError { error } => {
            handle_error(
                sampling_broadcast_sender,
                commitments_broadcast_sender,
                error,
            );
        }
    }
}

fn handle_error(
    sampling_broadcast_sender: &broadcast::Sender<SamplingEvent>,
    commitments_broadcast_sender: &broadcast::Sender<CommitmentsEvent>,
    error: SamplingError,
) {
    if let Err(e) = sampling_broadcast_sender.send(SamplingEvent::SamplingError {
        error: error.clone(),
    }) {
        error! {"Error in internal broadcast of sampling error: {e:?}"};
    }
    if let Err(e) = commitments_broadcast_sender.send(CommitmentsEvent::CommitmentsError { error })
    {
        error! {"Error in internal broadcast of commitments error: {e:?}"};
    }
}

async fn handle_request(
    sampling_broadcast_sender: &broadcast::Sender<SamplingEvent>,
    commitments_broadcast_sender: &broadcast::Sender<CommitmentsEvent>,
    request_receiver: Receiver<BehaviourSampleReq>,
    response_sender: Sender<BehaviourSampleRes>,
) {
    match request_receiver.await {
        Ok(BehaviourSampleReq::Share { blob_id, share_idx }) => {
            handle_incoming_share_request(
                sampling_broadcast_sender,
                response_sender,
                blob_id,
                share_idx,
            )
            .await;
        }
        Ok(BehaviourSampleReq::Commitments { blob_id }) => {
            handle_incoming_commitments_request(
                commitments_broadcast_sender,
                response_sender,
                blob_id,
            )
            .await;
        }
        Err(e) => {
            error!("Request receiver was closed: {e}");
        }
    }
}

#[expect(clippy::cognitive_complexity, reason = "share flow in one place")]
async fn handle_incoming_share_request(
    sampling_broadcast_sender: &broadcast::Sender<SamplingEvent>,
    response_sender: Sender<BehaviourSampleRes>,
    blob_id: BlobId,
    share_idx: u16,
) {
    let (sampling_response_sender, mut sampling_response_receiver) = mpsc::channel(1);

    if let Err(e) = sampling_broadcast_sender.send(SamplingEvent::SamplingRequest {
        blob_id,
        share_idx,
        response_sender: sampling_response_sender,
    }) {
        error!("Error in internal broadcast of sampling request: {e:?}");
        sampling_response_receiver.close();
    }

    if let Some(maybe_share) = sampling_response_receiver.recv().await {
        let result = match maybe_share {
            Some(share) => BehaviourSampleRes::SamplingSuccess {
                blob_id,
                subnetwork_id: share.share_idx,
                share: Box::new(share),
            },
            None => BehaviourSampleRes::SampleNotFound {
                blob_id,
                subnetwork_id: share_idx,
            },
        };

        if response_sender.send(result).is_err() {
            error!("Error sending sampling success response");
        }
    } else if response_sender
        .send(BehaviourSampleRes::SampleNotFound {
            blob_id,
            subnetwork_id: share_idx,
        })
        .is_err()
    {
        error!("Error sending sampling success response");
    }
}

#[expect(clippy::cognitive_complexity, reason = "commitments flow in one place")]
async fn handle_incoming_commitments_request(
    commitments_broadcast_sender: &broadcast::Sender<CommitmentsEvent>,
    response_sender: Sender<BehaviourSampleRes>,
    blob_id: BlobId,
) {
    let (commitments_response_sender, mut commitments_response_receiver) = mpsc::channel(1);

    if let Err(e) = commitments_broadcast_sender.send(CommitmentsEvent::CommitmentsRequest {
        blob_id,
        response_sender: commitments_response_sender,
    }) {
        error!("Error in internal broadcast of commitments request: {e:?}");
        commitments_response_receiver.close();
    }

    if let Some(maybe_share) = commitments_response_receiver.recv().await {
        let result = maybe_share.map_or(
            BehaviourSampleRes::CommitmentsNotFound { blob_id },
            |commitments| BehaviourSampleRes::CommitmentsSuccess {
                blob_id,
                commitments: Box::new(commitments),
            },
        );

        if response_sender.send(result).is_err() {
            error!("Error sending commitments response");
        }
    } else if response_sender
        .send(BehaviourSampleRes::CommitmentsNotFound { blob_id })
        .is_err()
    {
        error!("Error sending commitments response");
    }
}

pub(crate) async fn handle_sample_request(
    sampling_request_channel: &UnboundedSender<BlobId>,
    blob_id: BlobId,
) {
    if let Err(SendError(blob_id)) = sampling_request_channel.send(blob_id) {
        error!("Error requesting samples for blob_id: {blob_id:?}");
    }
}

pub(crate) async fn handle_historic_sample_request<Membership>(
    historic_sample_request_channel: &UnboundedSender<SampleArgs<Membership>>,
    blob_ids: HashSet<BlobId>,
    block_number: BlockNumber,
    membership: Membership,
) {
    if let Err(SendError((blob_id, block_number, _))) =
        historic_sample_request_channel.send((blob_ids, block_number, membership))
    {
        error!("Error requesting historic sample for blob_id: {blob_id:?}, block_number: {block_number:?}");
    }
}

pub(crate) async fn handle_commitments_request(
    commitments_request_channel: &UnboundedSender<BlobId>,
    blob_id: BlobId,
) {
    if let Err(SendError(blob_id)) = commitments_request_channel.send(blob_id) {
        error!("Error requesting commitments for blob_id: {blob_id:?}");
    }
}

pub(crate) async fn handle_monitor_command<Stats: Debug>(
    monitor_request_channel: &UnboundedSender<ConnectionMonitorCommand<Stats>>,
    command: ConnectionMonitorCommand<Stats>,
) {
    if let Err(SendError(cmd)) = monitor_request_channel.send(command) {
        error!("Channel closed when sending command to monitor: {cmd:?}");
    }
}

pub(crate) async fn handle_balancer_command<Stats: Debug>(
    balancer_request_channel: &UnboundedSender<ConnectionBalancerCommand<Stats>>,
    response_sender: oneshot::Sender<Stats>,
) {
    if let Err(SendError(cmd)) =
        balancer_request_channel.send(ConnectionBalancerCommand::Stats(response_sender))
    {
        error!("Error stats request: {cmd:?}");
    }
}
