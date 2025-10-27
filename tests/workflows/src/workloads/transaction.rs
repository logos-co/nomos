use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures::FutureExt as _;
use nomos_core::mantle::ops::channel::ChannelId;
use rand::{RngCore as _, thread_rng};
use testing_framework_core::{
    adjust_timeout,
    scenario::{
        BoxFuture, CONSENSUS_TRANSACTIONS_TOTAL, ClientError, Expectation, ExpectationError,
        Metrics, NodeClient, RunContext, Workload, WorkloadError,
    },
};

mod tx_factory {
    use ed25519_dalek::{Signer as _, SigningKey};
    use nomos_core::{
        mantle::{
            MantleTx, Op, OpProof, SignedMantleTx, Transaction as _,
            ledger::Tx as LedgerTx,
            ops::channel::{ChannelId, MsgId, inscribe::InscriptionOp},
        },
        proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
    };

    #[must_use]
    pub fn create_inscription_transaction_with_id(id: ChannelId) -> SignedMantleTx {
        let signing_key = SigningKey::from_bytes(&[0u8; 32]);
        let signer = signing_key.verifying_key();

        let inscription_op = InscriptionOp {
            channel_id: id,
            inscription: format!("Test channel inscription {id:?}").into_bytes(),
            parent: MsgId::root(),
            signer,
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelInscribe(inscription_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            storage_gas_price: 0,
            execution_gas_price: 0,
        };

        let tx_hash = mantle_tx.hash();
        let signature = signing_key.sign(&tx_hash.as_signing_bytes());

        SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::Ed25519Sig(signature)],
            DummyZkSignature::prove(&ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("valid transaction")
    }
}

fn client_err(err: &ClientError) -> WorkloadError {
    WorkloadError::new(err.to_string())
}

#[derive(Clone)]
pub struct TxWorkloadConfig {
    pub transactions: usize,
    pub interval: Duration,
}

impl Default for TxWorkloadConfig {
    fn default() -> Self {
        Self {
            transactions: 5,
            interval: Duration::from_millis(200),
        }
    }
}

pub struct TxWorkload {
    config: TxWorkloadConfig,
    baseline_counter: Arc<AtomicU64>,
}

impl Clone for TxWorkload {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            baseline_counter: Arc::clone(&self.baseline_counter),
        }
    }
}

impl TxWorkload {
    #[must_use]
    pub fn new(config: TxWorkloadConfig) -> Self {
        Self {
            config,
            baseline_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    fn record_baseline(&self, metrics: &Metrics) {
        if let Ok(value) = metrics.consensus_transactions_total() {
            self.baseline_counter.store(value as u64, Ordering::SeqCst);
        }
    }

    async fn submit_transactions(&self, client: &dyn NodeClient) -> Result<(), WorkloadError> {
        for _ in 0..self.config.transactions {
            let tx = tx_factory::create_inscription_transaction_with_id(random_channel_id());
            client
                .submit_transaction(tx)
                .await
                .map_err(|err| client_err(&err))?;

            if !self.config.interval.is_zero() {
                tokio::time::sleep(self.config.interval).await;
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn expect_inclusion(&self, expected: usize) -> TxInclusionExpectation {
        TxInclusionExpectation {
            baseline_counter: Arc::clone(&self.baseline_counter),
            expected,
        }
    }
}

impl Workload for TxWorkload {
    fn name(&self) -> &'static str {
        "tx_workload"
    }

    fn configure_metrics(&self, metrics: &Metrics) {
        self.record_baseline(metrics);
    }

    fn start<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), WorkloadError>> {
        async move {
            let handle = ctx
                .validators()
                .first()
                .ok_or_else(|| WorkloadError::new("no validator available"))?;
            let client = handle.client();
            self.submit_transactions(client).await
        }
        .boxed()
    }
}

fn random_channel_id() -> ChannelId {
    let mut channel_id_bytes = [0u8; 32];
    thread_rng().fill_bytes(&mut channel_id_bytes);
    ChannelId::from(channel_id_bytes)
}

pub struct TxInclusionExpectation {
    baseline_counter: Arc<AtomicU64>,
    expected: usize,
}

impl Expectation for TxInclusionExpectation {
    fn name(&self) -> &'static str {
        "tx_inclusion_expectation"
    }

    fn evaluate<'a>(&'a self, ctx: &'a RunContext) -> BoxFuture<'a, Result<(), ExpectationError>> {
        async move { self.wait_for_expected_delta(ctx).await }.boxed()
    }
}

impl TxInclusionExpectation {
    async fn wait_for_expected_delta(&self, ctx: &RunContext) -> Result<(), ExpectationError> {
        let baseline = self.baseline_counter.load(Ordering::SeqCst) as f64;
        let expected = self.expected as f64;
        let wait_future = wait_for_tx_delta(ctx.metrics(), baseline, expected);

        match tokio::time::timeout(adjust_timeout(Duration::from_secs(30)), wait_future).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(Self::timeout_error(ctx, baseline, expected)?),
        }
    }

    fn timeout_error(
        ctx: &RunContext,
        baseline: f64,
        expected: f64,
    ) -> Result<ExpectationError, ExpectationError> {
        let total = ctx
            .metrics()
            .consensus_transactions_total()
            .map_err(|err| ExpectationError::new(err.to_string()))?;

        Ok(ExpectationError::new(format!(
            "prometheus metric `{CONSENSUS_TRANSACTIONS_TOTAL}` delta {} below expected {}; baseline={baseline}, total={total}",
            total - baseline,
            expected
        )))
    }
}

async fn wait_for_tx_delta(
    metrics: &Metrics,
    baseline: f64,
    expected: f64,
) -> Result<f64, ExpectationError> {
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    loop {
        let total = metrics
            .consensus_transactions_total()
            .map_err(|err| ExpectationError::new(err.to_string()))?;
        println!("total={total}, baseline={baseline}, expected={expected}");
        if total - baseline >= expected {
            return Ok(total);
        }
        ticker.tick().await;
    }
}
