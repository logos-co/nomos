use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU64,
    time::Duration,
};

use async_trait::async_trait;
use integration_configs::topology::configs::wallet::WalletAccount;
use nomos_core::mantle::{
    GenesisTx as _, Note, SignedMantleTx, Transaction as _, Utxo, tx_builder::MantleTxBuilder,
};
use testing_framework_core::{
    scenario::{DynError, RunContext, RunMetrics, Workload as ScenarioWorkload},
    topology::{GeneratedNodeConfig, GeneratedTopology},
};
use tokio::time::sleep;
use zksign::{PublicKey, SecretKey};

#[derive(Clone)]
pub struct Workload {
    txs_per_block: NonZeroU64,
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
        "wallet_tx_workload"
    }

    fn init(
        &mut self,
        descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        let wallet_accounts = descriptors.config().wallet().accounts.clone();
        if wallet_accounts.is_empty() {
            return Err("wallet workload requires seeded accounts".into());
        }

        let reference_node = descriptors
            .validators()
            .first()
            .or_else(|| descriptors.executors().first())
            .ok_or("wallet workload requires at least one node in the topology")?;

        let utxo_map = wallet_utxo_map(reference_node);
        let accounts = wallet_accounts
            .into_iter()
            .filter_map(|account| {
                utxo_map
                    .get(&account.public_key())
                    .copied()
                    .map(|utxo| WalletInput { account, utxo })
            })
            .collect::<Vec<_>>();

        if accounts.is_empty() {
            return Err("wallet workload could not match any accounts to genesis UTXOs".into());
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
            return Err("wallet workload has no available accounts".into());
        }

        let (requested, interval) = submission_plan(workload.txs_per_block, ctx)?;
        let planned = requested.min(workload.accounts.len());
        if planned == 0 {
            return Err("wallet workload scheduled zero transactions".into());
        }

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
    let client = ctx
        .random_node_client()
        .ok_or("wallet workload requires at least one API client")?;

    let signed_tx = build_wallet_transaction(input)?;
    client.submit_transaction(&signed_tx).await?;

    Ok(())
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
    .map_err(|err| format!("wallet workload could not sign transaction: {err}"))?;

    SignedMantleTx::new(mantle_tx, Vec::new(), signature)
        .map_err(|err| format!("wallet workload constructed invalid transaction: {err}").into())
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

fn submission_plan(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
) -> Result<(usize, Duration), DynError> {
    let blocks = ctx.expected_blocks().max(1);
    let total = blocks
        .checked_mul(txs_per_block.get())
        .ok_or("wallet workload transaction total exceeds capacity")?;

    let total_usize = usize::try_from(total).map_err(|_| "wallet workload total too large")?;

    let interval = if total == 0 {
        Duration::ZERO
    } else {
        let secs = ctx.run_duration().as_secs_f64();
        if !secs.is_finite() || secs <= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(secs / total as f64)
        }
    };

    Ok((total_usize, interval))
}
