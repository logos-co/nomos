use std::num::{NonZeroU64, NonZeroUsize};

use integration_configs::topology::configs::network::Libp2pNetworkLayout;
use testing_framework_core::{scenario::ScenarioBuilder, topology::configs::wallet::WalletConfig};

use crate::{
    expectations::ConsensusLiveness,
    workloads::{channel, transaction, wallet},
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
non_zero_rate_fn!(
    wallet_rate_checked,
    "wallet transaction rate must be non-zero"
);

pub trait ScenarioBuilderExt: Sized {
    fn topology(self) -> TopologyConfigurator;
    fn transactions(self) -> TransactionFlowBuilder;
    fn da(self) -> DataAvailabilityFlowBuilder;
    fn wallet_transactions(self) -> WalletFlowBuilder;
    fn expect_consensus_liveness(self) -> ScenarioBuilder;
    fn initialize_wallet(self, total_funds: u64, users: usize) -> ScenarioBuilder;
}

impl ScenarioBuilderExt for ScenarioBuilder {
    fn topology(self) -> TopologyConfigurator {
        TopologyConfigurator { builder: self }
    }

    fn transactions(self) -> TransactionFlowBuilder {
        TransactionFlowBuilder::new(self)
    }

    fn da(self) -> DataAvailabilityFlowBuilder {
        DataAvailabilityFlowBuilder::new(self)
    }

    fn wallet_transactions(self) -> WalletFlowBuilder {
        WalletFlowBuilder::new(self)
    }

    fn expect_consensus_liveness(self) -> ScenarioBuilder {
        self.with_expectation(ConsensusLiveness)
    }

    fn initialize_wallet(self, total_funds: u64, users: usize) -> ScenarioBuilder {
        let user_count = NonZeroUsize::new(users).expect("wallet user count must be non-zero");
        let wallet = WalletConfig::uniform(total_funds, user_count);
        self.with_wallet_config(wallet)
    }
}

pub struct TopologyConfigurator {
    builder: ScenarioBuilder,
}

impl TopologyConfigurator {
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
    pub fn apply(self) -> ScenarioBuilder {
        self.builder
    }
}

pub struct TransactionFlowBuilder {
    builder: ScenarioBuilder,
    rate: NonZeroU64,
}

impl TransactionFlowBuilder {
    const fn default_rate() -> NonZeroU64 {
        transaction_rate_checked(1)
    }

    const fn new(builder: ScenarioBuilder) -> Self {
        Self {
            builder,
            rate: Self::default_rate(),
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
    pub fn apply(mut self) -> ScenarioBuilder {
        let workload = transaction::Workload::with_rate(self.rate.get())
            .expect("transaction rate must be non-zero");
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

pub struct DataAvailabilityFlowBuilder {
    builder: ScenarioBuilder,
    channel_rate: NonZeroU64,
    blob_rate: NonZeroU64,
}

impl DataAvailabilityFlowBuilder {
    const fn default_channel_rate() -> NonZeroU64 {
        channel_rate_checked(1)
    }

    const fn default_blob_rate() -> NonZeroU64 {
        blob_rate_checked(1)
    }

    const fn new(builder: ScenarioBuilder) -> Self {
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
    pub fn apply(mut self) -> ScenarioBuilder {
        let count = (self.channel_rate.get() * self.blob_rate.get()) as usize;
        let workload = channel::Workload::with_channel_count(count.max(1));
        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}

pub struct WalletFlowBuilder {
    builder: ScenarioBuilder,
    rate: NonZeroU64,
}

impl WalletFlowBuilder {
    const fn default_rate() -> NonZeroU64 {
        wallet_rate_checked(1)
    }

    const fn new(builder: ScenarioBuilder) -> Self {
        Self {
            builder,
            rate: Self::default_rate(),
        }
    }

    #[must_use]
    pub const fn rate(mut self, rate: u64) -> Self {
        self.rate = wallet_rate_checked(rate);
        self
    }

    #[must_use]
    pub fn apply(mut self) -> ScenarioBuilder {
        let workload = wallet::Workload::with_rate(self.rate.get())
            .expect("wallet transaction rate must be non-zero");

        self.builder = self.builder.with_workload(workload);
        self.builder
    }
}
