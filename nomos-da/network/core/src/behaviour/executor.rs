use std::time::Duration;

use libp2p::{identity::Keypair, swarm::NetworkBehaviour, PeerId};
use subnetworks_assignations::MembershipHandler;

use crate::{
    address_book::AddressBook,
    maintenance::{
        balancer::{ConnectionBalancer, ConnectionBalancerBehaviour},
        monitor::{ConnectionMonitor, ConnectionMonitorBehaviour},
    },
    protocols::{
        dispersal::{
            executor::behaviour::DispersalExecutorBehaviour,
            validator::behaviour::DispersalValidatorBehaviour,
        },
        replication::behaviour::ReplicationBehaviour,
        sampling::behaviour::SamplingBehaviour,
    },
};

/// Aggregated `NetworkBehaviour` composed of:
/// * Sampling
/// * Executor dispersal
/// * Validator dispersal
/// * Replication WARNING: Order of internal protocols matters as the first one
///   will be polled first until return a `Poll::Pending`.
/// 1) Sampling is the crucial one as we have to be responsive for consensus.
/// 2) Dispersal so we do not bottleneck executors.
/// 3) Replication is the least important (and probably the least used), it is
///    also dependant of dispersal.
#[derive(NetworkBehaviour)]
pub struct ExecutorBehaviour<Balancer, Monitor, Membership>
where
    Balancer: ConnectionBalancer,
    Monitor: ConnectionMonitor,
    Membership: MembershipHandler,
{
    sampling: SamplingBehaviour<Membership>,
    executor_dispersal: DispersalExecutorBehaviour<Membership>,
    validator_dispersal: DispersalValidatorBehaviour<Membership>,
    replication: ReplicationBehaviour<Membership>,
    balancer: ConnectionBalancerBehaviour<Balancer>,
    monitor: ConnectionMonitorBehaviour<Monitor>,
}

impl<Balancer, Monitor, Membership> ExecutorBehaviour<Balancer, Monitor, Membership>
where
    Balancer: ConnectionBalancer,
    Monitor: ConnectionMonitor,
    Membership: MembershipHandler + Clone + Send + 'static,
    <Membership as MembershipHandler>::NetworkId: Send,
{
    pub fn new(
        key: &Keypair,
        membership: Membership,
        addresses: AddressBook,
        balancer: Balancer,
        monitor: Monitor,
        redial_cooldown: Duration,
    ) -> Self {
        let peer_id = PeerId::from_public_key(&key.public());
        Self {
            sampling: SamplingBehaviour::new(peer_id, membership.clone(), addresses.clone()),
            executor_dispersal: DispersalExecutorBehaviour::new(
                peer_id,
                membership.clone(),
                addresses.clone(),
            ),
            validator_dispersal: DispersalValidatorBehaviour::new(membership.clone()),
            replication: ReplicationBehaviour::new(peer_id, membership),
            balancer: ConnectionBalancerBehaviour::new(addresses, balancer),
            monitor: ConnectionMonitorBehaviour::new(monitor, redial_cooldown),
        }
    }

    pub fn update_membership(&mut self, membership: Membership) {
        // TODO: share membership
        self.sampling.update_membership(membership.clone());
        self.executor_dispersal
            .update_membership(membership.clone());
        self.replication.update_membership(membership);
    }

    pub const fn sampling_behaviour(&self) -> &SamplingBehaviour<Membership> {
        &self.sampling
    }

    pub const fn dispersal_executor_behaviour(&self) -> &DispersalExecutorBehaviour<Membership> {
        &self.executor_dispersal
    }

    pub const fn dispersal_validator_behaviour(&self) -> &DispersalValidatorBehaviour<Membership> {
        &self.validator_dispersal
    }

    pub const fn replication_behaviour(&self) -> &ReplicationBehaviour<Membership> {
        &self.replication
    }

    pub fn sampling_behaviour_mut(&mut self) -> &mut SamplingBehaviour<Membership> {
        &mut self.sampling
    }

    pub fn dispersal_executor_behaviour_mut(
        &mut self,
    ) -> &mut DispersalExecutorBehaviour<Membership> {
        &mut self.executor_dispersal
    }

    pub fn dispersal_validator_behaviour_mut(
        &mut self,
    ) -> &mut DispersalValidatorBehaviour<Membership> {
        &mut self.validator_dispersal
    }

    pub fn replication_behaviour_mut(&mut self) -> &mut ReplicationBehaviour<Membership> {
        &mut self.replication
    }

    pub fn monitor_behaviour_mut(&mut self) -> &mut ConnectionMonitorBehaviour<Monitor> {
        &mut self.monitor
    }

    pub fn balancer_behaviour_mut(&mut self) -> &mut ConnectionBalancerBehaviour<Balancer> {
        &mut self.balancer
    }
}
