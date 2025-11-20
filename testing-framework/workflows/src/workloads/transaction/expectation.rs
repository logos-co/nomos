use std::{
    collections::HashSet,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use nomos_core::mantle::{AuthenticatedMantleTx as _, ops::Op};
use testing_framework_core::scenario::{DynError, Expectation, RunContext};
use thiserror::Error;
use tokio::sync::broadcast;

use super::workload::{TxPlanError, planned_channel_ids, planned_transaction_totals};

const MIN_INCLUSION_RATIO: f64 = 0.8;

#[derive(Debug, Error)]
enum TxInclusionError {
    #[error("expected transaction count exceeds u64 capacity")]
    ExpectedCountOverflow,
    #[error("validator transaction total {actual} below expected {expected}")]
    BelowExpectation { actual: u64, expected: u64 },
    #[error("tx inclusion expectation not prepared")]
    NotCaptured,
}

impl TxInclusionExpectation {
    pub const NAME: &'static str = "tx_inclusion_expectation";

    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64) -> Self {
        Self {
            txs_per_block,
            capture_state: None,
        }
    }
}

struct CaptureState {
    observed: Arc<AtomicU64>,
    expected: u64,
}

pub struct TxInclusionExpectation {
    txs_per_block: NonZeroU64,
    capture_state: Option<CaptureState>,
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

        let (_, expected) = planned_transaction_totals(self.txs_per_block, ctx)
            .map_err(|err| match err {
                TxPlanError::TotalOverflow | TxPlanError::CapacityOverflow => {
                    TxInclusionError::ExpectedCountOverflow
                }
            })
            .map_err(DynError::from)?;

        let planned = planned_channel_ids(expected);
        let relevant_ids = Arc::new(planned.into_iter().collect::<HashSet<_>>());
        let observed = Arc::new(AtomicU64::new(0));

        let receiver = ctx.block_feed().subscribe();
        let spawn_ids = Arc::clone(&relevant_ids);
        let spawn_observed = Arc::clone(&observed);

        tokio::spawn(async move {
            let mut receiver = receiver;
            loop {
                match receiver.recv().await {
                    Ok(record) => {
                        for tx in record.block.transactions() {
                            for op in &tx.mantle_tx().ops {
                                if let Op::ChannelInscribe(inscribe) = op
                                    && spawn_ids.contains(&inscribe.channel_id)
                                {
                                    spawn_observed.fetch_add(1, Ordering::Relaxed);
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
            expected: relevant_ids.len() as u64,
        });

        Ok(())
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        let state = self
            .capture_state
            .as_ref()
            .ok_or(TxInclusionError::NotCaptured)
            .map_err(DynError::from)?;

        let observed = state.observed.load(Ordering::Relaxed);
        let required = ((state.expected as f64) * MIN_INCLUSION_RATIO).ceil() as u64;

        if observed >= required {
            Ok(())
        } else {
            Err(TxInclusionError::BelowExpectation {
                actual: observed,
                expected: required,
            }
            .into())
        }
    }
}
