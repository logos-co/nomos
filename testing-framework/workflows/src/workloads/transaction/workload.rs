use std::{
    collections::{HashMap, VecDeque},
    num::{NonZeroU64, NonZeroUsize},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use integration_configs::topology::configs::wallet::WalletAccount;
use nomos_core::mantle::{
    GenesisTx as _, Note, SignedMantleTx, Transaction as _, Utxo, tx_builder::MantleTxBuilder,
};
use testing_framework_core::{
    scenario::{DynError, Expectation, RunContext, RunMetrics, Workload as ScenarioWorkload},
    topology::{GeneratedNodeConfig, GeneratedTopology},
};
use tokio::time::sleep;
use zksign::{PublicKey, SecretKey};

use super::expectation::TxInclusionExpectation;
use crate::workloads::util::submit_transaction_via_cluster;

#[derive(Clone)]
pub struct Workload {
    txs_per_block: NonZeroU64,
    user_limit: Option<NonZeroUsize>,
    accounts: Vec<WalletInput>,
}

#[derive(Clone)]
struct WalletInput {
    account: WalletAccount,
    utxo: Utxo,
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "tx_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(TxInclusionExpectation::new(
            self.txs_per_block,
            self.user_limit,
        ))]
    }

    fn init(
        &mut self,
        descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        let wallet_accounts = descriptors.config().wallet().accounts.clone();
        if wallet_accounts.is_empty() {
            return Err("transaction workload requires seeded accounts".into());
        }

        let reference_node = descriptors
            .validators()
            .first()
            .or_else(|| descriptors.executors().first())
            .ok_or("transaction workload requires at least one node in the topology")?;

        let utxo_map = wallet_utxo_map(reference_node);
        let mut accounts = wallet_accounts
            .into_iter()
            .filter_map(|account| {
                utxo_map
                    .get(&account.public_key())
                    .copied()
                    .map(|utxo| WalletInput { account, utxo })
            })
            .collect::<Vec<_>>();

        apply_user_limit(&mut accounts, self.user_limit);

        if accounts.is_empty() {
            return Err(
                "transaction workload could not match any accounts to genesis UTXOs".into(),
            );
        }

        self.accounts = accounts;
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        Submission::new(self, ctx)?.execute().await
    }
}

impl Workload {
    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64) -> Self {
        Self {
            txs_per_block,
            user_limit: None,
            accounts: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_rate(txs_per_block: u64) -> Option<Self> {
        NonZeroU64::new(txs_per_block).map(Self::new)
    }

    #[must_use]
    pub const fn txs_per_block(&self) -> NonZeroU64 {
        self.txs_per_block
    }

    #[must_use]
    pub const fn with_user_limit(mut self, user_limit: Option<NonZeroUsize>) -> Self {
        self.user_limit = user_limit;
        self
    }
}

impl Default for Workload {
    fn default() -> Self {
        Self::new(NonZeroU64::new(1).expect("non-zero"))
    }
}

struct Submission<'a> {
    plan: VecDeque<WalletInput>,
    ctx: &'a RunContext,
    interval: Duration,
}

impl<'a> Submission<'a> {
    fn new(workload: &Workload, ctx: &'a RunContext) -> Result<Self, DynError> {
        if workload.accounts.is_empty() {
            return Err("transaction workload has no available accounts".into());
        }

        let (planned, interval) =
            submission_plan(workload.txs_per_block, ctx, workload.accounts.len())?;

        let plan = workload
            .accounts
            .iter()
            .take(planned)
            .cloned()
            .collect::<VecDeque<_>>();

        Ok(Self {
            plan,
            ctx,
            interval,
        })
    }

    async fn execute(mut self) -> Result<(), DynError> {
        while let Some(input) = self.plan.pop_front() {
            submit_wallet_transaction(self.ctx, &input).await?;

            if !self.interval.is_zero() {
                sleep(self.interval).await;
            }
        }

        Ok(())
    }
}

async fn submit_wallet_transaction(ctx: &RunContext, input: &WalletInput) -> Result<(), DynError> {
    let signed_tx = Arc::new(build_wallet_transaction(input)?);
    submit_transaction_via_cluster(ctx, signed_tx).await
}

fn build_wallet_transaction(input: &WalletInput) -> Result<SignedMantleTx, DynError> {
    let builder = MantleTxBuilder::new()
        .add_ledger_input(input.utxo)
        .add_ledger_output(Note::new(input.utxo.note.value, input.account.public_key()));

    let mantle_tx = builder.build();
    let tx_hash = mantle_tx.hash();

    let signature = SecretKey::multi_sign(
        std::slice::from_ref(&input.account.secret_key),
        tx_hash.as_ref(),
    )
    .map_err(|err| format!("transaction workload could not sign transaction: {err}"))?;

    SignedMantleTx::new(mantle_tx, Vec::new(), signature).map_err(|err| {
        format!("transaction workload constructed invalid transaction: {err}").into()
    })
}

fn wallet_utxo_map(node: &GeneratedNodeConfig) -> HashMap<PublicKey, Utxo> {
    let genesis_tx = node.general.consensus_config.genesis_tx.clone();
    let ledger_tx = genesis_tx.mantle_tx().ledger_tx.clone();
    let tx_hash = ledger_tx.hash();

    ledger_tx
        .outputs
        .iter()
        .enumerate()
        .map(|(idx, note)| (note.pk, Utxo::new(tx_hash, idx, *note)))
        .collect()
}

fn apply_user_limit<T>(items: &mut Vec<T>, user_limit: Option<NonZeroUsize>) {
    if let Some(limit) = user_limit {
        let allowed = limit.get().min(items.len());
        items.truncate(allowed);
    }
}

pub(super) fn limited_user_count(user_limit: Option<NonZeroUsize>, available: usize) -> usize {
    user_limit.map_or(available, |limit| limit.get().min(available))
}

pub(super) fn submission_plan(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
    available_accounts: usize,
) -> Result<(usize, Duration), DynError> {
    if available_accounts == 0 {
        return Err("transaction workload scheduled zero transactions".into());
    }

    let run_secs = ctx.run_duration().as_secs_f64();
    let block_secs = ctx
        .run_metrics()
        .block_interval_hint()
        .unwrap_or_else(|| ctx.run_duration())
        .as_secs_f64();

    let expected_blocks = run_secs / block_secs;
    let requested = (expected_blocks * txs_per_block.get() as f64)
        .floor()
        .clamp(0.0, u64::MAX as f64) as u64;

    let planned = requested.min(available_accounts as u64) as usize;
    if planned == 0 {
        return Err("transaction workload scheduled zero transactions".into());
    }

    let interval = Duration::from_secs_f64(run_secs / planned as f64);
    Ok((planned, interval))
}
