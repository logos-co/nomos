use std::{
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

use integration_configs::topology::configs::network::Libp2pNetworkLayout;
use testing_framework_core::{
    scenario::{Builder as CoreScenarioBuilder, NodeControlCapability},
    topology::configs::wallet::WalletConfig,
};

use crate::{
    expectations::ConsensusLiveness,
    workloads::{channel, chaos::RandomRestartWorkload, transaction},
};

macro_rules! non_zero_rate_fn {
    ($name:ident, $message:literal) => {
        const fn $name(rate: u64) -> NonZeroU64 {
            match NonZeroU64::new(rate) {
                Some(value) => value,
                None => panic!($message),
            }
        }
    };
}

non_zero_rate_fn!(
    transaction_rate_checked,
    "transaction rate must be non-zero"
);
non_zero_rate_fn!(channel_rate_checked, "channel rate must be non-zero");
non_zero_rate_fn!(blob_rate_checked, "blob rate must be non-zero");

pub trait ScenarioBuilderExt<Caps>: Sized {
    fn topology(self) -> TopologyConfigurator<Caps>;
    fn transactions(self) -> TransactionFlowBuilder<Caps>;
    fn da(self) -> DataAvailabilityFlowBuilder<Caps>;
    #[must_use]
    fn expect_consensus_liveness(self) -> Self;
    #[must_use]
    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self;
}

impl<Caps> ScenarioBuilderExt<Caps> for CoreScenarioBuilder<Caps> {
    fn topology(self) -> TopologyConfigurator<Caps> {
        TopologyConfigurator { builder: self }
    }

    fn transactions(self) -> TransactionFlowBuilder<Caps> {
        TransactionFlowBuilder::new(self)
    }

    fn da(self) -> DataAvailabilityFlowBuilder<Caps> {
        DataAvailabilityFlowBuilder::new(self)
    }

    fn expect_consensus_liveness(self) -> Self {
        self.with_expectation(ConsensusLiveness)
    }

    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self {
        let user_count = NonZeroUsize::new(users).expect("wallet user count must be non-zero");
        let wallet = WalletConfig::uniform(total_funds, user_count);
        self.with_wallet_config(wallet)
    }
}

pub struct TopologyConfigurator<Caps> {
    builder: CoreScenarioBuilder<Caps>,
}

impl<Caps> TopologyConfigurator<Caps> {
    #[must_use]
    pub fn validators(mut self, count: usize) -> Self {
        self.builder = self
            .builder
            .map_topology(|topology| topology.with_validator_count(count));
        self
    }

    #[must_use]
    pub fn executors(mut self, count: usize) -> Self {
        self.builder = self
            .builder
            .map_topology(|topology| topology.with_executor_count(count));
        self
    }

    #[must_use]
    pub fn network_star(mut self) -> Self {
        self.builder = self
            .builder
            .map_topology(|topology| topology.with_network_layout(Libp2pNetworkLayout::Star));
        self
    }

    #[must_use]
    pub fn apply(self) -> CoreScenarioBuilder<Caps> {
        self.builder
    }
}

pub struct TransactionFlowBuilder<Caps> {
    builder: CoreScenarioBuilder<Caps>,
    rate: NonZeroU64,
    users: Option<NonZeroUsize>,
}

impl<Caps> TransactionFlowBuilder<Caps> {
    const fn default_rate() -> NonZeroU64 {
        transaction_rate_checked(1)
    }

    const fn new(builder: CoreScenarioBuilder<Caps>) -> Self {
        Self {
            builder,
            rate: Self::default_rate(),
            users: None,
        }
    }

    #[must_use]
    pub const fn rate(mut self, rate: u64) -> Self {
        self.rate = transaction_rate_checked(rate);
        self
    }

    #[must_use]
    pub const fn rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.rate = rate;
        self
    }

    #[must_use]
    pub const fn users(mut self, users: usize) -> Self {
        match NonZeroUsize::new(users) {
            Some(value) => self.users = Some(value),
            None => panic!("transaction user count must be non-zero"),
        }
        self
    }

    #[must_use]
    pub fn apply(mut self) -> CoreScenarioBuilder<Caps> {
        let workload = transaction::Workload::with_rate(self.rate.get())
            .expect("transaction rate must be non-zero")
            .with_user_limit(self.users);
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

pub struct DataAvailabilityFlowBuilder<Caps> {
    builder: CoreScenarioBuilder<Caps>,
    channel_rate: NonZeroU64,
    blob_rate: NonZeroU64,
}

impl<Caps> DataAvailabilityFlowBuilder<Caps> {
    const fn default_channel_rate() -> NonZeroU64 {
        channel_rate_checked(1)
    }

    const fn default_blob_rate() -> NonZeroU64 {
        blob_rate_checked(1)
    }

    const fn new(builder: CoreScenarioBuilder<Caps>) -> Self {
        Self {
            builder,
            channel_rate: Self::default_channel_rate(),
            blob_rate: Self::default_blob_rate(),
        }
    }

    #[must_use]
    pub const fn rate(mut self, rate: u64) -> Self {
        self.channel_rate = channel_rate_checked(rate);
        self
    }

    #[must_use]
    pub const fn rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.channel_rate = rate;
        self
    }

    #[must_use]
    pub const fn blob_rate(mut self, rate: u64) -> Self {
        self.blob_rate = blob_rate_checked(rate);
        self
    }

    #[must_use]
    pub const fn blob_rate_per_block(mut self, rate: NonZeroU64) -> Self {
        self.blob_rate = rate;
        self
    }

    #[must_use]
    pub fn apply(mut self) -> CoreScenarioBuilder<Caps> {
        let count = (self.channel_rate.get() * self.blob_rate.get()) as usize;
        let workload = channel::Workload::with_channel_count(count.max(1));
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

pub trait ChaosBuilderExt: Sized {
    fn chaos_random_restart(self) -> ChaosRestartBuilder;
}

impl ChaosBuilderExt for CoreScenarioBuilder<NodeControlCapability> {
    fn chaos_random_restart(self) -> ChaosRestartBuilder {
        ChaosRestartBuilder::new(self)
    }
}

pub struct ChaosRestartBuilder {
    builder: CoreScenarioBuilder<NodeControlCapability>,
    min_delay: Duration,
    max_delay: Duration,
    include_validators: bool,
    include_executors: bool,
}

impl ChaosRestartBuilder {
    #[expect(
        clippy::missing_const_for_fn,
        reason = "Scenario builder contains runtime-only structures"
    )]
    fn new(builder: CoreScenarioBuilder<NodeControlCapability>) -> Self {
        Self {
            builder,
            min_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            include_validators: true,
            include_executors: true,
        }
    }

    #[must_use]
    pub fn min_delay(mut self, delay: Duration) -> Self {
        assert!(!delay.is_zero(), "chaos restart min delay must be non-zero");
        self.min_delay = delay;
        self
    }

    #[must_use]
    pub fn max_delay(mut self, delay: Duration) -> Self {
        assert!(!delay.is_zero(), "chaos restart max delay must be non-zero");
        self.max_delay = delay;
        self
    }

    #[must_use]
    #[expect(
        clippy::missing_const_for_fn,
        reason = "builder mutates runtime-only configuration"
    )]
    pub fn include_validators(mut self, enabled: bool) -> Self {
        self.include_validators = enabled;
        self
    }

    #[must_use]
    #[expect(
        clippy::missing_const_for_fn,
        reason = "builder mutates runtime-only configuration"
    )]
    pub fn include_executors(mut self, enabled: bool) -> Self {
        self.include_executors = enabled;
        self
    }

    #[must_use]
    pub fn apply(mut self) -> CoreScenarioBuilder<NodeControlCapability> {
        assert!(
            self.min_delay <= self.max_delay,
            "chaos restart min delay must not exceed max delay"
        );
        assert!(
            self.include_validators || self.include_executors,
            "chaos restart requires at least one node group"
        );

        let workload = RandomRestartWorkload::new(
            self.min_delay,
            self.max_delay,
            self.include_validators,
            self.include_executors,
        );
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}
