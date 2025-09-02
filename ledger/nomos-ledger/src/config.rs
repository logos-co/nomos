use std::num::{NonZero, NonZeroU64};

use cryptarchia_engine::{Epoch, Slot};
use nomos_core::sdp;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Config {
    pub epoch_config: cryptarchia_engine::EpochConfig,
    pub consensus_config: cryptarchia_engine::Config,
    pub service_params: sdp::ServiceParameters,
    pub min_stake: sdp::MinStake,
}

impl Config {
    #[must_use]
    pub fn base_period_length(&self) -> NonZero<u64> {
        self.consensus_config.base_period_length()
    }

    #[must_use]
    pub fn epoch_length(&self) -> u64 {
        self.epoch_config
            .epoch_length(self.consensus_config.base_period_length())
    }

    #[must_use]
    pub fn nonce_snapshot(&self, epoch: Epoch) -> Slot {
        let offset = self.base_period_length().get().saturating_mul(
            u64::from(NonZeroU64::from(
                self.epoch_config.epoch_period_nonce_buffer,
            ))
            .saturating_add(u64::from(NonZeroU64::from(
                self.epoch_config.epoch_stake_distribution_stabilization,
            ))),
        );
        let base =
            u64::from(u32::from(epoch).saturating_sub(1)).saturating_mul(self.epoch_length());
        base.saturating_add(offset).into()
    }

    #[must_use]
    pub fn stake_distribution_snapshot(&self, epoch: Epoch) -> Slot {
        (u64::from(u32::from(epoch) - 1) * self.epoch_length()).into()
    }

    #[must_use]
    pub fn epoch(&self, slot: Slot) -> Epoch {
        self.epoch_config
            .epoch(slot, self.consensus_config.base_period_length())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use cryptarchia_engine::EpochConfig;
    use nomos_core::sdp::{MinStake, ServiceParameters};
    #[test]
    fn epoch_snapshots() {
        let config = super::Config {
            epoch_config: EpochConfig {
                epoch_stake_distribution_stabilization: NonZero::new(3u8).unwrap(),
                epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
                epoch_period_nonce_stabilization: NonZero::new(4).unwrap(),
            },
            consensus_config: cryptarchia_engine::Config {
                security_param: NonZero::new(5).unwrap(),
                active_slot_coeff: 0.5,
            },
            service_params: ServiceParameters {
                lock_period: 10,
                inactivity_period: 20,
                retention_period: 100,
                timestamp: 0,
            },
            min_stake: MinStake {
                threshold: 1,
                timestamp: 0,
            },
        };
        assert_eq!(config.epoch_length(), 100);
        assert_eq!(config.nonce_snapshot(1.into()), 60.into());
        assert_eq!(config.nonce_snapshot(2.into()), 160.into());
        assert_eq!(config.stake_distribution_snapshot(1.into()), 0.into());
        assert_eq!(config.stake_distribution_snapshot(2.into()), 100.into());
    }

    #[test]
    fn slot_to_epoch() {
        let config = super::Config {
            epoch_config: EpochConfig {
                epoch_stake_distribution_stabilization: NonZero::new(3u8).unwrap(),
                epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
                epoch_period_nonce_stabilization: NonZero::new(4).unwrap(),
            },
            consensus_config: cryptarchia_engine::Config {
                security_param: NonZero::new(5).unwrap(),
                active_slot_coeff: 0.5,
            },
            service_params: ServiceParameters {
                lock_period: 10,
                inactivity_period: 20,
                retention_period: 100,
                timestamp: 0,
            },
            min_stake: MinStake {
                threshold: 1,
                timestamp: 0,
            },
        };
        assert_eq!(config.epoch(1.into()), 0.into());
        assert_eq!(config.epoch(100.into()), 1.into());
        assert_eq!(config.epoch(101.into()), 1.into());
        assert_eq!(config.epoch(200.into()), 2.into());
    }
}
