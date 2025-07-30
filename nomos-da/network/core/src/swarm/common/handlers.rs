use libp2p::PeerId;
use log::{debug, error};
use nomos_da_messages::replication::ReplicationRequest;
use subnetworks_assignations::MembershipHandler;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    maintenance::monitor::{ConnectionMonitor, ConnectionMonitorBehaviour},
    protocols::{
        dispersal::validator::behaviour::DispersalEvent,
        replication::behaviour::{ReplicationBehaviour, ReplicationEvent},
        sampling::SamplingEvent,
    },
    SubnetworkId,
};

pub async fn handle_validator_dispersal_event<Membership>(
    validation_events_sender: &UnboundedSender<DispersalEvent>,
    replication_behaviour: &mut ReplicationBehaviour<Membership>,
    event: DispersalEvent,
) where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    // Send message for replication
    if let Err(e) = validation_events_sender.send(event.clone()) {
        error!("Error sending blob to validation: {e:?}");
    }

    match event {
        DispersalEvent::IncomingShare(share) => {
            replication_behaviour.send_message(&ReplicationRequest::new_share(*share));
        }
        DispersalEvent::IncomingTx(signed_mantle_tx) => {
            replication_behaviour.send_message(&ReplicationRequest::new_tx(*signed_mantle_tx));
        }
        DispersalEvent::DispersalError { .. } => {} // Do not replicate errors.
    }
}

pub async fn handle_sampling_event(
    sampling_events_sender: &UnboundedSender<SamplingEvent>,
    event: SamplingEvent,
) {
    if let Err(e) = sampling_events_sender.send(event) {
        debug!("Error distributing sampling message internally: {e:?}");
    }
}

pub async fn handle_replication_event(
    validation_events_sender: &UnboundedSender<DispersalEvent>,
    event: ReplicationEvent,
) {
    if let ReplicationEvent::IncomingMessage { message, .. } = event {
        let dispersal_event = match message.as_ref() {
            ReplicationRequest::Share(share_request) => {
                DispersalEvent::new_share(share_request.share.clone())
            }
            ReplicationRequest::Tx(_signed_mantle_tx) => todo!(),
        };
        if let Err(e) = validation_events_sender.send(dispersal_event) {
            error!("Error sending blob to validation: {e:?}");
        }
    }
}

pub fn monitor_event<Monitor: ConnectionMonitor>(
    monitor_behaviour: &mut ConnectionMonitorBehaviour<Monitor>,
    event: Monitor::Event,
) {
    monitor_behaviour.record_event(event);
}
