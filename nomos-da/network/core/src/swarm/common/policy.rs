// std
// crates
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
// internal
use super::balancer::{
    ConnectionDeviation, SubnetworkConnectionPolicy, SubnetworkDeviation, SubnetworkStats,
};
use crate::{
    swarm::common::monitor::{PeerHealthPolicy, PeerStats},
    SubnetworkId,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DAConnectionPolicySettings {
    pub min_dispersal_peers: usize,
    pub min_replication_peers: usize,
    pub max_dispersal_failures: usize,
    pub max_sampling_failures: usize,
    pub max_replication_failures: usize,
    pub malicious_threshold: usize,
}

pub struct DAConnectionPolicy<Membership> {
    settings: DAConnectionPolicySettings,
    membership: Membership,
    local_peer_id: PeerId,
}

impl<Membership> DAConnectionPolicy<Membership> {
    pub fn new(
        settings: DAConnectionPolicySettings,
        membership: Membership,
        local_peer_id: PeerId,
    ) -> Self {
        Self {
            settings,
            membership,
            local_peer_id,
        }
    }
}

impl<Membership> PeerHealthPolicy for DAConnectionPolicy<Membership> {
    type PeerStats = PeerStats;

    fn is_peer_malicious(&self, stats: &Self::PeerStats) -> bool {
        stats.dispersal_failures_rate >= self.settings.malicious_threshold
            || stats.sampling_failures_rate >= self.settings.malicious_threshold
            || stats.replication_failures_rate >= self.settings.malicious_threshold
    }

    fn is_peer_unhealthy(&self, stats: &Self::PeerStats) -> bool {
        stats.dispersal_failures_rate >= self.settings.max_dispersal_failures
            || stats.sampling_failures_rate >= self.settings.max_sampling_failures
            || stats.replication_failures_rate >= self.settings.max_replication_failures
    }
}

impl<Membership> SubnetworkConnectionPolicy for DAConnectionPolicy<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>,
{
    fn connection_number_deviation(
        &self,
        subnetwork_id: &SubnetworkId,
        stats: &SubnetworkStats,
    ) -> SubnetworkDeviation {
        let is_member = self
            .membership
            .is_member_of(&self.local_peer_id, subnetwork_id);

        let required_connections = if is_member {
            self.settings
                .min_dispersal_peers
                .max(self.settings.min_replication_peers)
        } else {
            self.settings.min_dispersal_peers
        };

        let total_missing = required_connections.saturating_sub(stats.inbound)
            + required_connections.saturating_sub(stats.outbound);

        SubnetworkDeviation {
            outbound: ConnectionDeviation::Missing(total_missing), // All missing are counted as outbound.
        }
    }
}
