use std::num::NonZero;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    // The k parameter in the Common Prefix property.
    // Blocks deeper than k are generally considered stable and forks deeper than that
    // trigger the additional fork selection rule, which is however only expected to be used
    // during bootstrapping.
    pub security_param: NonZero<u32>,
    // f, the rate of occupied slots
    pub active_slot_coeff: f64,
}

impl Config {
    #[must_use]
    pub fn base_period_length(&self) -> NonZero<u64> {
        let inner = (f64::from(self.security_param.get()) / self.active_slot_coeff).floor();
        // `active_slot_coeff` cannot be negative, so result will be a valid `u64`,
        // without risks of losing sign info.
        #[expect(clippy::cast_possible_truncation)]
        #[expect(clippy::cast_sign_loss)]
        NonZero::new(inner as u64)
            .expect("base_period_length with proper configuration should never be zero")
    }

    // return the number of slots required to have great confidence at least k
    // blocks have been produced
    #[must_use]
    pub fn s(&self) -> u64 {
        self.base_period_length().get().saturating_mul(3)
    }
}
