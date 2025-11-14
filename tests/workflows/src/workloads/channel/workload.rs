use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use executor_http_client::ExecutorHttpClient;
use nomos_core::{
    block::Block,
    da::BlobId,
    mantle::{
        AuthenticatedMantleTx as _, SignedMantleTx,
        ops::{
            Op,
            channel::{ChannelId, MsgId},
        },
    },
};
use nomos_node::HeaderId;
use rand::{RngCore as _, thread_rng};
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{DynError, Expectation, RunContext, Workload as ScenarioWorkload},
};
use tokio::time::sleep;
use tracing::warn;

use crate::util::tx;

const POLL_DELAY_MS: u64 = 200;
// The DA encoder chunkifies payloads into 31-byte field elements; keeping the
// blob length as a multiple of 31 avoids panics inside kzgrs when padding the
// final chunk.
const FIELD_ELEMENT_BYTES: usize = 31;
const BLOB_CHUNKS: usize = 4;
const BLOB_BYTES: usize = FIELD_ELEMENT_BYTES * BLOB_CHUNKS;
const TEST_KEY_BYTES: [u8; 32] = [0u8; 32];
const BLOB_RETRY_DELAY: Duration = Duration::from_secs(1);
const BLOB_RETRIES: usize = 5;

#[derive(Clone, Default)]
pub struct Workload {
    state: Arc<ChannelWorkloadState>,
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "channel_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(ChannelWorkloadExpectation::new(Arc::clone(
            &self.state,
        )))]
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        self.state.reset();
        run_channel_flow(ctx).await?;
        self.state.mark_success();
        Ok(())
    }
}

async fn run_channel_flow(ctx: &RunContext) -> Result<(), DynError> {
    let validator = ctx
        .node_clients()
        .validator_clients()
        .first()
        .cloned()
        .ok_or_else(|| "channel workload requires at least one validator".to_owned())?;
    let executor = ctx
        .node_clients()
        .executor_clients()
        .first()
        .cloned()
        .ok_or_else(|| "channel workload requires at least one executor".to_owned())?;

    let channel_id = random_channel_id();
    let tx = tx::create_inscription_transaction_with_id(channel_id);
    validator.submit_transaction(&tx).await?;

    let inscription_id = wait_for_inscription(&validator, channel_id).await?;
    let blob_id = publish_blob(&executor, channel_id, inscription_id).await?;
    let _blob_msg = wait_for_blob(&executor, channel_id, blob_id).await?;

    Ok(())
}

async fn wait_for_inscription(
    client: &ApiClient,
    channel_id: ChannelId,
) -> Result<MsgId, DynError> {
    wait_for_channel_op(client, move |op| {
        if let Op::ChannelInscribe(inscribe) = op
            && inscribe.channel_id == channel_id
        {
            Some(inscribe.id())
        } else {
            None
        }
    })
    .await
}

async fn wait_for_blob(
    client: &ApiClient,
    channel_id: ChannelId,
    blob_id: BlobId,
) -> Result<MsgId, DynError> {
    wait_for_channel_op(client, move |op| {
        if let Op::ChannelBlob(blob_op) = op
            && blob_op.channel == channel_id
            && blob_op.blob == blob_id
        {
            Some(blob_op.id())
        } else {
            None
        }
    })
    .await
}

async fn wait_for_channel_op<F>(client: &ApiClient, mut matcher: F) -> Result<MsgId, DynError>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    let mut scanned = HashSet::new();
    loop {
        let tip = client.consensus_info().await?.tip;
        if let Some(msg_id) = scan_chain(client, tip, &mut scanned, &mut matcher).await? {
            return Ok(msg_id);
        }
        sleep(Duration::from_millis(POLL_DELAY_MS)).await;
    }
}

async fn scan_chain<F>(
    client: &ApiClient,
    start: HeaderId,
    scanned: &mut HashSet<HeaderId>,
    matcher: &mut F,
) -> Result<Option<MsgId>, DynError>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    let mut cursor = Some(start);
    while let Some(header) = cursor {
        if !scanned.insert(header) {
            break;
        }

        let Some(block) = client.storage_block(&header).await? else {
            break;
        };

        if let Some(msg_id) = find_channel_op(&block, matcher) {
            return Ok(Some(msg_id));
        }

        cursor = Some(block.header().parent());
    }

    Ok(None)
}

fn find_channel_op<F>(block: &Block<SignedMantleTx>, matcher: &mut F) -> Option<MsgId>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    for tx in block.transactions() {
        for op in &tx.mantle_tx().ops {
            if let Some(msg_id) = matcher(op) {
                return Some(msg_id);
            }
        }
    }

    None
}

async fn publish_blob(
    executor: &ApiClient,
    channel_id: ChannelId,
    parent_msg: MsgId,
) -> Result<BlobId, DynError> {
    let executor_url = executor.base_url().clone();
    let signer = SigningKey::from_bytes(&TEST_KEY_BYTES).verifying_key();
    let mut data = vec![0u8; BLOB_BYTES];
    thread_rng().fill_bytes(&mut data);

    let client = ExecutorHttpClient::new(None);
    let mut attempt = 1;
    loop {
        match client
            .publish_blob(
                executor_url.clone(),
                channel_id,
                parent_msg,
                signer,
                data.clone(),
            )
            .await
        {
            Ok(blob_id) => return Ok(blob_id),
            Err(err) if attempt < BLOB_RETRIES => {
                warn!(
                    attempt,
                    retries = BLOB_RETRIES,
                    "failed to publish blob: {err}; retrying"
                );
                attempt += 1;
                sleep(BLOB_RETRY_DELAY).await;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn random_channel_id() -> ChannelId {
    let mut channel_id_bytes = [0u8; 32];
    thread_rng().fill_bytes(&mut channel_id_bytes);
    ChannelId::from(channel_id_bytes)
}

#[derive(Default)]
struct ChannelWorkloadState {
    completed: AtomicBool,
}

impl ChannelWorkloadState {
    fn reset(&self) {
        self.completed.store(false, Ordering::SeqCst);
    }

    fn mark_success(&self) {
        self.completed.store(true, Ordering::SeqCst);
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }
}

struct ChannelWorkloadExpectation {
    state: Arc<ChannelWorkloadState>,
}

impl ChannelWorkloadExpectation {
    const fn new(state: Arc<ChannelWorkloadState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Expectation for ChannelWorkloadExpectation {
    fn name(&self) -> &'static str {
        "channel_workload_completed"
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        if self.state.is_completed() {
            Ok(())
        } else {
            Err("channel workload did not complete channel flow".into())
        }
    }
}
