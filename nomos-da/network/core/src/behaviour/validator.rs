use std::time::Duration;

use libp2p::{identity::Keypair, swarm::NetworkBehaviour, PeerId};
use subnetworks_assignations::MembershipHandler;

use crate::{
    addressbook::AddressBookHandler,
    maintenance::{
        balancer::{ConnectionBalancer, ConnectionBalancerBehaviour},
        monitor::{ConnectionMonitor, ConnectionMonitorBehaviour},
    },
    protocols::{
        dispersal::validator::behaviour::DispersalValidatorBehaviour,
        replication::behaviour::{ReplicationBehaviour, ReplicationConfig},
        sampling::behaviour::{SamplingBehaviour, SubnetsConfig},
    },
};

/// Aggregated `NetworkBehaviour` composed of:
/// * Sampling
/// * Dispersal
/// * Replication WARNING: Order of internal protocols matters as the first one
///   will be polled first until return a `Poll::Pending`.
/// 1) Sampling is the crucial one as we have to be responsive for consensus.
/// 2) Dispersal so we do not bottleneck executors.
/// 3) Replication is the least important (and probably the least used), it is
///    also dependant of dispersal.
#[derive(NetworkBehaviour)]
pub struct ValidatorBehaviour<Balancer, Monitor, Membership, Addressbook>
where
    Balancer: ConnectionBalancer,
    Monitor: ConnectionMonitor,
    Membership: MembershipHandler,
    Addressbook: AddressBookHandler,
{
    sampling: SamplingBehaviour<Membership, Addressbook>,
    dispersal: DispersalValidatorBehaviour<Membership>,
    replication: ReplicationBehaviour<Membership>,
    balancer: ConnectionBalancerBehaviour<Balancer, Addressbook>,
    monitor: ConnectionMonitorBehaviour<Monitor>,
}

impl<Balancer, BalancerStats, Monitor, Membership, Addressbook>
    ValidatorBehaviour<Balancer, Monitor, Membership, Addressbook>
where
    Balancer: ConnectionBalancer<Stats = BalancerStats>,
    Monitor: ConnectionMonitor,
    Membership: MembershipHandler + Clone + Send + 'static,
    <Membership as MembershipHandler>::NetworkId: Send,
    Addressbook: AddressBookHandler + Clone + Send + 'static,
{
    pub fn new(
        key: &Keypair,
        membership: Membership,
        addressbook: Addressbook,
        balancer: Balancer,
        monitor: Monitor,
        redial_cooldown: Duration,
        replication_config: ReplicationConfig,
        subnets_config: SubnetsConfig,
        refresh_signal: impl futures::Stream<Item = ()> + Send + 'static,
    ) -> Self {
        let peer_id = PeerId::from_public_key(&key.public());
        Self {
            sampling: SamplingBehaviour::new(
                peer_id,
                membership.clone(),
                addressbook.clone(),
                subnets_config,
                refresh_signal,
            ),
            dispersal: DispersalValidatorBehaviour::new(membership.clone()),
            replication: ReplicationBehaviour::new(replication_config, peer_id, membership),
            balancer: ConnectionBalancerBehaviour::new(addressbook, balancer),
            monitor: ConnectionMonitorBehaviour::new(monitor, redial_cooldown),
        }
    }

    pub const fn sampling_behaviour(&self) -> &SamplingBehaviour<Membership, Addressbook> {
        &self.sampling
    }

    pub const fn dispersal_behaviour(&self) -> &DispersalValidatorBehaviour<Membership> {
        &self.dispersal
    }

    pub const fn replication_behaviour(&self) -> &ReplicationBehaviour<Membership> {
        &self.replication
    }

    pub const fn sampling_behaviour_mut(
        &mut self,
    ) -> &mut SamplingBehaviour<Membership, Addressbook> {
        &mut self.sampling
    }

    pub const fn dispersal_behaviour_mut(
        &mut self,
    ) -> &mut DispersalValidatorBehaviour<Membership> {
        &mut self.dispersal
    }

    pub const fn replication_behaviour_mut(&mut self) -> &mut ReplicationBehaviour<Membership> {
        &mut self.replication
    }

    pub const fn monitor_behaviour_mut(&mut self) -> &mut ConnectionMonitorBehaviour<Monitor> {
        &mut self.monitor
    }

    pub const fn monitor_behavior(&self) -> &ConnectionMonitorBehaviour<Monitor> {
        &self.monitor
    }

    pub const fn balancer_behaviour_mut(
        &mut self,
    ) -> &mut ConnectionBalancerBehaviour<Balancer, Addressbook> {
        &mut self.balancer
    }

    pub const fn balancer_behaviour(&self) -> &ConnectionBalancerBehaviour<Balancer, Addressbook> {
        &self.balancer
    }
}
