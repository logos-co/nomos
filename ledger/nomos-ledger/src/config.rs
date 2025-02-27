use cryptarchia_engine::{Epoch, Slot};
use std::num::NonZero;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Config {
    pub epoch_config: cryptarchia_engine::EpochConfig,
    pub consensus_config: cryptarchia_engine::Config,
}

impl Config {
    pub fn base_period_length(&self) -> NonZero<u64> {
        self.consensus_config.base_period_length()
    }

    pub fn epoch_length(&self) -> u64 {
        self.epoch_config
            .epoch_length(self.consensus_config.base_period_length())
    }

    pub fn nonce_snapshot(&self, epoch: Epoch) -> Slot {
        let offset = self.base_period_length().get().saturating_mul(
            (self.epoch_config.epoch_period_nonce_buffer.get()
                + self
                    .epoch_config
                    .epoch_stake_distribution_stabilization
                    .get()) as u64,
        );
        let base = (u32::from(epoch) - 1) as u64 * self.epoch_length();
        (base + offset).into()
    }

    pub fn stake_distribution_snapshot(&self, epoch: Epoch) -> Slot {
        ((u32::from(epoch) - 1) as u64 * self.epoch_length()).into()
    }

    pub fn epoch(&self, slot: Slot) -> Epoch {
        self.epoch_config
            .epoch(slot, self.consensus_config.base_period_length())
    }
}

#[cfg(test)]
mod tests {
    use cryptarchia_engine::EpochConfig;
    use std::num::NonZero;
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
        };
        assert_eq!(config.epoch(1.into()), 0.into());
        assert_eq!(config.epoch(100.into()), 1.into());
        assert_eq!(config.epoch(101.into()), 1.into());
        assert_eq!(config.epoch(200.into()), 2.into());
    }
}
