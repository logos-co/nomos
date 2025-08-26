use futures::channel::oneshot;
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
    swarm::{DispersalValidationError, DispersalValidatorEvent},
    SubnetworkId,
};

pub async fn handle_validator_dispersal_event<Membership>(
    validation_events_sender: &UnboundedSender<DispersalValidatorEvent>,
    replication_behaviour: &mut ReplicationBehaviour<Membership>,
    event: DispersalEvent,
) where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    let (sender, receiver) = oneshot::channel();
    let validation_event = DispersalValidatorEvent {
        event: event.clone(),
        sender: Some(sender),
    };

    if let Err(e) = validation_events_sender.send(validation_event) {
        error!("Error sending blob to validation: {e:?}");
    }

    let Ok(validation_result) = receiver.await else {
        error!("Error receiving dispersal validation result");
        return;
    };

    // Do not replicate if validation fails.
    if matches!(validation_result, Err(DispersalValidationError)) {
        error!("Error validating dispersal event: {event:?}");
        return;
    }

    // Send message for replication
    match event {
        DispersalEvent::IncomingShare(share) => {
            replication_behaviour.send_message(&ReplicationRequest::from(*share));
        }
        DispersalEvent::IncomingTx(signed_mantle_tx) => {
            replication_behaviour.send_message(&ReplicationRequest::from(*signed_mantle_tx.1));
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

pub async fn handle_replication_event<Membership>(
    validation_events_sender: &UnboundedSender<DispersalValidatorEvent>,
    membership: &Membership,
    peer_id: &<Membership as MembershipHandler>::Id,
    event: ReplicationEvent,
) where
    Membership: MembershipHandler + Send + Sync,
    <Membership as MembershipHandler>::Id: Send + Sync,
{
    if let ReplicationEvent::IncomingMessage { message, .. } = event {
        let dispersal_event = match *message {
            ReplicationRequest::Share(share_request) => DispersalEvent::from(share_request.share),
            ReplicationRequest::Tx(tx) => {
                let assignations = membership.membership(peer_id).len();
                DispersalEvent::from((assignations as u16, tx))
            }
        };

        let validation_event = DispersalValidatorEvent {
            event: dispersal_event,
            sender: None,
        };

        if let Err(e) = validation_events_sender.send(validation_event) {
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
