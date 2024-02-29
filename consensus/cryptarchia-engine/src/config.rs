use crate::{Epoch, Slot};

#[derive(Clone, Debug, PartialEq)]
pub struct TimeConfig {
    // How long a slot lasts in seconds
    pub slot_duration: u64,
    // Start of the first epoch, in unix timestamp second precision
    pub chain_start_time: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    // The k parameter in the Common Prefix property.
    // Blocks deeper than k are generally considered stable and forks deeper than that
    // trigger the additional fork selection rule, which is however only expected to be used
    // during bootstrapping.
    pub security_param: u32,
    // f, the rate of occupied slots
    pub active_slot_coeff: f64,
    // The stake distribution is always taken at the beginning of the previous epoch.
    // This parameters controls how many slots to wait for it to be stabilized
    // The value is computed as epoch_stake_distribution_stabilization * int(floor(k / f))
    pub epoch_stake_distribution_stabilization: u8,
    // This parameter controls how many slots we wait after the stake distribution
    // snapshot has stabilized to take the nonce snapshot.
    pub epoch_period_nonce_buffer: u8,
    // This parameter controls how many slots we wait for the nonce snapshot to be considered
    // stabilized
    pub epoch_period_nonce_stabilization: u8,
    pub time: TimeConfig,
}
impl Config {
    pub fn time_config(&self) -> &TimeConfig {
        &self.time
    }

    pub fn base_period_length(&self) -> u64 {
        (f64::from(self.security_param) / self.active_slot_coeff).floor() as u64
    }

    // return the number of slots required to have great confidence at least k blocks have been produced
    pub fn s(&self) -> u64 {
        self.base_period_length() * 3
    }

    pub fn epoch_length(&self) -> u64 {
        (self.epoch_stake_distribution_stabilization as u64
            + self.epoch_period_nonce_buffer as u64
            + self.epoch_period_nonce_stabilization as u64)
            * self.base_period_length()
    }

    pub fn nonce_snapshot(&self, epoch: Epoch) -> Slot {
        let offset = self.base_period_length()
            * (self.epoch_period_nonce_buffer + self.epoch_stake_distribution_stabilization) as u64;
        let base = u32::from(epoch) as u64 * self.epoch_length();
        (base + offset).into()
    }

    pub fn stake_distribution_snapshot(&self, epoch: Epoch) -> Slot {
        (u32::from(epoch) as u64 * self.epoch_length()).into()
    }
}
