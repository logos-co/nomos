use kzgrs_backend::common::share::DaShare;
use libp2p::PeerId;
use log::{debug, error};
use nomos_da_messages::replication;
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
    validation_events_sender: &UnboundedSender<DaShare>,
    replication_behaviour: &mut ReplicationBehaviour<Membership>,
    event: DispersalEvent,
) where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    // Send message for replication
    if let DispersalEvent::IncomingMessage { message } = event {
        let share_message = message.share;
        if let Err(e) = validation_events_sender.send(share_message.data.clone()) {
            error!("Error sending blob to validation: {e:?}");
        }
        replication_behaviour.send_message(&replication::ReplicationRequest::new(
            share_message,
            message.subnetwork_id,
        ));
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
    validation_events_sender: &UnboundedSender<DaShare>,
    event: ReplicationEvent,
) {
    if let ReplicationEvent::IncomingMessage { message, .. } = event {
        if let Err(e) = validation_events_sender.send(message.share.data) {
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
