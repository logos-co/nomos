use std::{
    collections::HashSet,
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use nomos_core::{header::HeaderId, mantle::AuthenticatedMantleTx as _};
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use thiserror::Error;
use tokio::sync::broadcast;
use zksign::PublicKey;

use super::workload::{limited_user_count, submission_plan};

const MIN_INCLUSION_RATIO: f64 = 0.5;

#[derive(Clone)]
pub struct TxInclusionExpectation {
    txs_per_block: NonZeroU64,
    user_limit: Option<NonZeroUsize>,
    capture_state: Option<CaptureState>,
}

#[derive(Clone)]
struct CaptureState {
    observed: Arc<AtomicU64>,
    expected: u64,
}

#[derive(Debug, Error)]
enum TxExpectationError {
    #[error("transaction workload requires seeded accounts")]
    MissingAccounts,
    #[error("transaction workload planned zero transactions")]
    NoPlannedTransactions,
    #[error("transaction inclusion expectation not captured")]
    NotCaptured,
    #[error("transaction inclusion observed {observed} below required {required}")]
    InsufficientInclusions { observed: u64, required: u64 },
}

impl TxInclusionExpectation {
    pub const NAME: &'static str = "tx_inclusion_expectation";

    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64, user_limit: Option<NonZeroUsize>) -> Self {
        Self {
            txs_per_block,
            user_limit,
            capture_state: None,
        }
    }
}

#[async_trait]
impl Expectation for TxInclusionExpectation {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        if self.capture_state.is_some() {
            return Ok(());
        }

        let wallet_accounts = ctx.descriptors().config().wallet().accounts.clone();
        if wallet_accounts.is_empty() {
            return Err(TxExpectationError::MissingAccounts.into());
        }

        let available = limited_user_count(self.user_limit, wallet_accounts.len());
        let (planned, _) = submission_plan(self.txs_per_block, ctx, available)?;
        if planned == 0 {
            return Err(TxExpectationError::NoPlannedTransactions.into());
        }

        let wallet_pks = wallet_accounts
            .into_iter()
            .take(planned)
            .map(|account| account.secret_key.to_public_key())
            .collect::<HashSet<PublicKey>>();

        let observed = Arc::new(AtomicU64::new(0));
        let receiver = ctx.block_feed().subscribe();
        let tracked_accounts = Arc::new(wallet_pks);
        let spawn_accounts = Arc::clone(&tracked_accounts);
        let spawn_observed = Arc::clone(&observed);

        tokio::spawn(async move {
            let mut receiver = receiver;
            let genesis_parent = HeaderId::from([0; 32]);
            loop {
                match receiver.recv().await {
                    Ok(record) => {
                        if record.block.header().parent_block() == genesis_parent {
                            continue;
                        }

                        for tx in record.block.transactions() {
                            for note in &tx.mantle_tx().ledger_tx.outputs {
                                if spawn_accounts.contains(&note.pk) {
                                    spawn_observed.fetch_add(1, Ordering::Relaxed);
                                    break;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        self.capture_state = Some(CaptureState {
            observed,
            expected: planned as u64,
        });

        Ok(())
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        let state = self
            .capture_state
            .as_ref()
            .ok_or(TxExpectationError::NotCaptured)?;

        let observed = state.observed.load(Ordering::Relaxed);
        let required = ((state.expected as f64) * MIN_INCLUSION_RATIO).ceil() as u64;

        if observed >= required {
            Ok(())
        } else {
            Err(TxExpectationError::InsufficientInclusions { observed, required }.into())
        }
    }
}
