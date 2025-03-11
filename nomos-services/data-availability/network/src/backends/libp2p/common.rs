use std::time::Duration;

use futures::StreamExt;
use kzgrs_backend::common::{
    blob::{DaBlob, DaLightBlob},
    ColumnIndex,
};
use libp2p::PeerId;
use log::error;
use nomos_core::da::BlobId;
use nomos_da_network_core::{
    maintenance::monitor::PeerCommand,
    protocols::sampling::{
        self,
        behaviour::{BehaviourSampleReq, BehaviourSampleRes, SamplingError},
    },
    swarm::{
        validator::ValidatorEventsStream, DAConnectionMonitorSettings, DAConnectionPolicySettings,
    },
    SubnetworkId,
};
use nomos_libp2p::{ed25519, secret_key_serde, Multiaddr};
use serde::{Deserialize, Serialize};
use tokio::sync::{
    broadcast, mpsc,
    mpsc::{error::SendError, UnboundedSender},
    oneshot,
};

pub(crate) const BROADCAST_CHANNEL_SIZE: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaNetworkBackendSettings<Membership> {
    // Identification Secp256k1 private key in Hex format (`0x123...abc`). Default random.
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    /// Membership of DA network `PoV` set
    pub membership: Membership,
    pub listening_address: Multiaddr,
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
}

/// Sampling events coming from da network
#[derive(Debug, Clone)]
pub enum SamplingEvent {
    /// A success sampling
    SamplingSuccess {
        blob_id: BlobId,
        light_blob: Box<DaLightBlob>,
    },
    /// Incoming sampling request
    SamplingRequest {
        blob_id: BlobId,
        column_idx: ColumnIndex,
        response_sender: mpsc::Sender<Option<DaLightBlob>>,
    },
    /// A failed sampling error
    SamplingError { error: SamplingError },
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
    validation_broadcast_sender: broadcast::Sender<DaBlob>,
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
                match sampling_event {
                    sampling::behaviour::SamplingEvent::SamplingSuccess{ blob_id, light_blob , .. } => {
                        if let Err(e) = sampling_broadcast_sender.send(SamplingEvent::SamplingSuccess {blob_id, light_blob}){
                            error!("Error in internal broadcast of sampling success: {e:?}");
                        }
                    }
                    sampling::behaviour::SamplingEvent::IncomingSample{request_receiver, response_sender} => {
                        if let Ok(BehaviourSampleReq { blob_id, column_idx }) = request_receiver.await {
                            let (sampling_response_sender, mut sampling_response_receiver) = mpsc::channel(1);

                            if let Err(e) = sampling_broadcast_sender
                                .send(SamplingEvent::SamplingRequest { blob_id, column_idx, response_sender: sampling_response_sender })
                            {
                                error!("Error in internal broadcast of sampling request: {e:?}");
                                sampling_response_receiver.close();
                            }

                            if let Some(maybe_blob) = sampling_response_receiver.recv().await {
                                let result = match maybe_blob {
                                    Some(blob) => BehaviourSampleRes::SamplingSuccess {
                                        blob_id,
                                        subnetwork_id: blob.column_idx,
                                        blob: Box::new(blob),
                                    },
                                    None => BehaviourSampleRes::SampleNotFound { blob_id, subnetwork_id: column_idx },
                                };

                                if response_sender.send(result).is_err() {
                                    error!("Error sending sampling success response");
                                }
                            } else if response_sender
                                .send(BehaviourSampleRes::SampleNotFound { blob_id, subnetwork_id: column_idx })
                                .is_err()
                            {
                                error!("Error sending sampling success response");
                            }
                        }
                    }
                    sampling::behaviour::SamplingEvent::SamplingError{ error  } => {
                        if let Err(e) = sampling_broadcast_sender.send(SamplingEvent::SamplingError {error}) {
                            error!{"Error in internal broadcast of sampling error: {e:?}"};
                        }
                    }}
            }
            Some(da_blob) = StreamExt::next(&mut validation_events_receiver) => {
                if let Err(error) = validation_broadcast_sender.send(da_blob) {
                    error!("Error in internal broadcast of validation for blob: {:?}", error.0);
                }
            }
        }
    }
}

pub(crate) async fn handle_sample_request(
    sampling_request_channel: &UnboundedSender<(SubnetworkId, BlobId)>,
    subnetwork_id: SubnetworkId,
    blob_id: BlobId,
) {
    if let Err(SendError((subnetwork_id, blob_id))) =
        sampling_request_channel.send((subnetwork_id, blob_id))
    {
        error!("Error requesting sample for subnetwork id : {subnetwork_id}, blob_id: {blob_id:?}");
    }
}

pub(crate) async fn handle_block_peer_request(
    monitor_request_channel: &UnboundedSender<PeerCommand>,
    peer_id: PeerId,
    response_sender: oneshot::Sender<bool>,
) {
    if let Err(SendError(cmd)) =
        monitor_request_channel.send(PeerCommand::Block(peer_id, response_sender))
    {
        error!("Error peer request: {cmd:?}");
    }
}

pub(crate) async fn handle_unblock_peer_request(
    monitor_request_channel: &UnboundedSender<PeerCommand>,
    peer_id: PeerId,
    response_sender: oneshot::Sender<bool>,
) {
    if let Err(SendError(cmd)) =
        monitor_request_channel.send(PeerCommand::Unblock(peer_id, response_sender))
    {
        error!("Error peer request: {cmd:?}");
    }
}
