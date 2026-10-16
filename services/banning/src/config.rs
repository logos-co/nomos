use std::{collections::HashSet, time::Duration};

use lb_libp2p::PeerId;
use lb_services_utils::overwatch::{RecoveryData, StorageRecoverySettings};
use lb_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::types::{BanScope, BanSource, OffenseKind};

/// The portion of banning configuration that can be enforced without a live
/// [`crate::BanningService`].
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfiguredBanPolicy {
    blacklisted_peers: HashSet<PeerId>,
}

impl ConfiguredBanPolicy {
    /// Derive the effective configured blacklist. Whitelist precedence is
    /// applied once here and reused by both the store and consumers.
    #[must_use]
    pub fn from_config(config: &BanningConfig) -> Self {
        Self {
            blacklisted_peers: config
                .blacklist
                .iter()
                .copied()
                .filter(|peer_id| !config.whitelist.contains(peer_id))
                .collect(),
        }
    }

    #[must_use]
    pub fn contains(&self, peer_id: &PeerId) -> bool {
        self.blacklisted_peers.contains(peer_id)
    }

    #[must_use]
    pub const fn blacklisted_peers(&self) -> &HashSet<PeerId> {
        &self.blacklisted_peers
    }

    #[must_use]
    pub fn record(
        &self,
        peer_id: PeerId,
        reported_at: std::time::SystemTime,
    ) -> Option<crate::BanRecord> {
        self.contains(&peer_id).then_some(crate::BanRecord {
            peer_id,
            source: BanSource::Configuration,
            scope: BanScope::Global,
            offense: OffenseKind::BlackListed,
            context: Some("configured blacklist".to_owned()),
            reported_at,
            expires_at: None,
        })
    }
}

/// Configuration for the in-memory banning service.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BanningConfig {
    /// Peers in this list are never applicable to a ban. Whitelist wins over
    /// the blacklist if a peer appears in both lists.
    pub whitelist: Vec<PeerId>,
    /// Peers in this list receive an explicit, global, non-expiring ban.
    pub blacklist: Vec<PeerId>,
    #[serde_as(as = "MinimalBoundedDuration<1, SECOND>")]
    pub expiry_check_interval: Duration,
    /// Runtime recovery cache used by the `BanningService` state operator.
    #[serde(skip)]
    pub recovery_data: RecoveryData,
}

impl BanningConfig {
    #[must_use]
    pub fn configured_ban_policy(&self) -> ConfiguredBanPolicy {
        ConfiguredBanPolicy::from_config(self)
    }
}

impl StorageRecoverySettings for BanningConfig {
    const RECOVERY_KEY_SUFFIX: &'static [u8] = b"banning";

    fn recovery_data(&self) -> &RecoveryData {
        &self.recovery_data
    }
}

impl Default for BanningConfig {
    fn default() -> Self {
        Self {
            whitelist: Vec::new(),
            blacklist: Vec::new(),
            expiry_check_interval: Duration::from_secs(5),
            recovery_data: RecoveryData::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_policy_applies_whitelist_precedence() {
        let whitelisted = PeerId::random();
        let blacklisted = PeerId::random();
        let config = BanningConfig {
            whitelist: vec![whitelisted],
            blacklist: vec![whitelisted, blacklisted],
            ..Default::default()
        };

        let policy = config.configured_ban_policy();

        assert!(!policy.contains(&whitelisted));
        assert!(policy.contains(&blacklisted));
        assert_eq!(policy.blacklisted_peers().len(), 1);
    }

    #[test]
    fn defaults_are_safe_and_serde_compatible() {
        let config: BanningConfig = serde_yaml::from_str("{}").expect("default config");
        assert_eq!(config.whitelist, []);
        assert_eq!(config.blacklist, []);
        assert_eq!(config.expiry_check_interval, Duration::from_secs(5));
    }
}
