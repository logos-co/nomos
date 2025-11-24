use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use executor_http_client::ExecutorHttpClient;
use nomos_core::{
    da::BlobId,
    mantle::ops::{
        Op,
        channel::{ChannelId, MsgId},
    },
};
use rand::{Rng as _, RngCore as _, seq::SliceRandom as _, thread_rng};
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{BlockRecord, DynError, Expectation, RunContext, Workload as ScenarioWorkload},
};
use tokio::{sync::broadcast, time::sleep};

use super::expectation::DaWorkloadExpectation;
use crate::{
    util::tx,
    workloads::util::{find_channel_op, submit_transaction_via_cluster},
};

const TEST_KEY_BYTES: [u8; 32] = [0u8; 32];
const DEFAULT_CHANNELS: usize = 1;
const MIN_BLOB_CHUNKS: usize = 1;
const MAX_BLOB_CHUNKS: usize = 8;
const PUBLISH_RETRIES: usize = 5;
const PUBLISH_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Workload {
    planned_channels: Arc<[ChannelId]>,
}

impl Default for Workload {
    fn default() -> Self {
        Self::with_channel_count(DEFAULT_CHANNELS)
    }
}

impl Workload {
    #[must_use]
    pub fn with_channel_count(count: usize) -> Self {
        assert!(count > 0, "da workload requires positive count");
        Self {
            planned_channels: Arc::from(planned_channel_ids(count)),
        }
    }

    fn plan(&self) -> Arc<[ChannelId]> {
        Arc::clone(&self.planned_channels)
    }
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "channel_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        let planned = self.plan().to_vec();
        vec![Box::new(DaWorkloadExpectation::new(planned))]
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let mut receiver = ctx.block_feed().subscribe();

        for channel_id in self.plan().iter().copied() {
            run_channel_flow(ctx, &mut receiver, channel_id).await?;
        }

        Ok(())
    }
}

async fn run_channel_flow(
    ctx: &RunContext,
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    channel_id: ChannelId,
) -> Result<(), DynError> {
    let tx = Arc::new(tx::create_inscription_transaction_with_id(channel_id));
    submit_transaction_via_cluster(ctx, Arc::clone(&tx)).await?;

    let inscription_id = wait_for_inscription(receiver, channel_id).await?;
    let blob_id = publish_blob(ctx, channel_id, inscription_id).await?;
    wait_for_blob(receiver, channel_id, blob_id).await?;
    Ok(())
}

async fn wait_for_inscription(
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    channel_id: ChannelId,
) -> Result<MsgId, DynError> {
    wait_for_channel_op(receiver, move |op| {
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
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    channel_id: ChannelId,
    blob_id: BlobId,
) -> Result<MsgId, DynError> {
    wait_for_channel_op(receiver, move |op| {
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

async fn wait_for_channel_op<F>(
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    mut matcher: F,
) -> Result<MsgId, DynError>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    loop {
        match receiver.recv().await {
            Ok(record) => {
                if let Some(msg_id) = find_channel_op(record.block.as_ref(), &mut matcher) {
                    return Ok(msg_id);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                return Err("block feed closed while waiting for channel operations".into());
            }
        }
    }
}

async fn publish_blob(
    ctx: &RunContext,
    channel_id: ChannelId,
    parent_msg: MsgId,
) -> Result<BlobId, DynError> {
    let executors = ctx.node_clients().executor_clients();
    if executors.is_empty() {
        return Err("da workload requires at least one executor".into());
    }

    let signer = SigningKey::from_bytes(&TEST_KEY_BYTES).verifying_key();
    let data = random_blob_payload();
    let client = ExecutorHttpClient::new(None);

    let mut candidates: Vec<&ApiClient> = executors.iter().collect();
    let mut last_err = None;
    for attempt in 1..=PUBLISH_RETRIES {
        candidates.shuffle(&mut thread_rng());
        for executor in &candidates {
            let executor_url = executor.base_url().clone();
            match client
                .publish_blob(executor_url, channel_id, parent_msg, signer, data.clone())
                .await
            {
                Ok(blob_id) => return Ok(blob_id),
                Err(err) => last_err = Some(err.into()),
            }
        }

        if attempt < PUBLISH_RETRIES {
            sleep(PUBLISH_RETRY_DELAY).await;
        }
    }

    Err(last_err.unwrap_or_else(|| "da workload could not publish blob".into()))
}

fn random_blob_payload() -> Vec<u8> {
    let mut rng = thread_rng();
    let chunks = rng.gen_range(MIN_BLOB_CHUNKS..=MAX_BLOB_CHUNKS);
    let mut data = vec![0u8; 31 * chunks];
    rng.fill_bytes(&mut data);
    data
}

fn planned_channel_ids(total: usize) -> Vec<ChannelId> {
    (0..total as u64)
        .map(deterministic_channel_id)
        .collect::<Vec<_>>()
}

fn deterministic_channel_id(index: u64) -> ChannelId {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(b"chn_wrkd");
    bytes[24..].copy_from_slice(&index.to_be_bytes());
    ChannelId::from(bytes)
}
