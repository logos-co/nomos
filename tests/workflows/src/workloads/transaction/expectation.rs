use std::{collections::HashSet, num::NonZeroU64, time::Duration};

use async_trait::async_trait;
use testing_framework_core::{
    adjust_timeout,
    nodes::ApiClient,
    scenario::{DynError, Expectation, RunContext},
};
use thiserror::Error;
use tokio::time;

#[derive(Debug, Error)]
enum TxInclusionError {
    #[error("tx inclusion expectation requires at least one node")]
    MissingNode,
    #[error("expected transaction count exceeds u64 capacity")]
    ExpectedCountOverflow,
    #[error("validator transaction total {actual} below expected {expected}")]
    BelowExpectation { actual: u64, expected: u64 },
}

const TX_INCLUSION_TIMEOUT: Duration = Duration::from_secs(30);
const TX_INCLUSION_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct TxInclusionExpectation {
    txs_per_block: NonZeroU64,
}

#[async_trait]
impl Expectation for TxInclusionExpectation {
    fn name(&self) -> &'static str {
        "tx_inclusion_expectation"
    }

    async fn evaluate(&self, ctx: &RunContext) -> Result<(), DynError> {
        self.wait_for_expected_transaction(ctx).await
    }
}

impl TxInclusionExpectation {
    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64) -> Self {
        Self { txs_per_block }
    }

    async fn wait_for_expected_transaction(&self, ctx: &RunContext) -> Result<(), DynError> {
        let client = ctx
            .random_node_client()
            .ok_or(TxInclusionError::MissingNode)
            .map_err(DynError::from)?;

        let blocks = ctx.expected_blocks().max(1);
        let expected = blocks
            .checked_mul(self.txs_per_block.get())
            .ok_or(TxInclusionError::ExpectedCountOverflow)
            .map_err(DynError::from)?;

        let timeout = adjust_timeout(TX_INCLUSION_TIMEOUT);
        let mut latest_total = 0u64;
        let wait_future = async {
            loop {
                latest_total = count_chain_transactions(client).await?;
                if latest_total >= expected {
                    return Ok(());
                }

                time::sleep(TX_INCLUSION_POLL_INTERVAL).await;
            }
        };

        time::timeout(timeout, wait_future)
            .await
            .unwrap_or_else(|_| {
                Err::<(), DynError>(
                    TxInclusionError::BelowExpectation {
                        actual: latest_total,
                        expected,
                    }
                    .into(),
                )
            })
    }
}

async fn count_chain_transactions(client: &ApiClient) -> Result<u64, DynError> {
    let mut current = client.consensus_info().await?.tip;
    let mut total = 0u64;
    let mut visited = HashSet::new();

    while visited.insert(current) {
        let Some(block) = client.storage_block(&current).await? else {
            break;
        };
        total += block.transactions().len() as u64;
        current = block.header().parent();
    }

    Ok(total)
}
