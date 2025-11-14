use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result};
use nomos_core::{block::Block, mantle::SignedMantleTx};
use nomos_node::HeaderId;
use tokio::{sync::broadcast, task::JoinHandle, time::sleep};
use tracing::{debug, error};

use super::context::CleanupGuard;
use crate::nodes::ApiClient;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct BlockFeed {
    inner: Arc<BlockFeedInner>,
}

struct BlockFeedInner {
    sender: broadcast::Sender<Arc<BlockRecord>>,
    stats: Arc<BlockStats>,
}

#[derive(Clone)]
pub struct BlockRecord {
    pub header: HeaderId,
    pub block: Arc<Block<SignedMantleTx>>,
}

pub struct BlockFeedTask {
    handle: JoinHandle<()>,
}

impl BlockFeed {
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BlockRecord>> {
        self.inner.sender.subscribe()
    }

    #[must_use]
    pub fn stats(&self) -> Arc<BlockStats> {
        Arc::clone(&self.inner.stats)
    }

    fn ingest(&self, header: HeaderId, block: Block<SignedMantleTx>) {
        self.inner.stats.record_block(&block);
        let record = Arc::new(BlockRecord {
            header,
            block: Arc::new(block),
        });

        let _ = self.inner.sender.send(record);
    }
}

impl BlockFeedTask {
    #[must_use]
    pub const fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }
}

pub async fn spawn_block_feed(client: ApiClient) -> Result<(BlockFeed, BlockFeedTask)> {
    let (sender, _) = broadcast::channel(1024);
    let feed = BlockFeed {
        inner: Arc::new(BlockFeedInner {
            sender,
            stats: Arc::new(BlockStats::default()),
        }),
    };

    let mut scanner = BlockScanner::new(client, feed.clone());
    scanner.catch_up().await?;

    let handle = tokio::spawn(async move { scanner.run().await });

    Ok((feed, BlockFeedTask::new(handle)))
}

struct BlockScanner {
    client: ApiClient,
    feed: BlockFeed,
    seen: HashSet<HeaderId>,
}

impl BlockScanner {
    fn new(client: ApiClient, feed: BlockFeed) -> Self {
        Self {
            client,
            feed,
            seen: HashSet::new(),
        }
    }

    async fn run(&mut self) {
        loop {
            if let Err(err) = self.catch_up().await {
                error!(%err, "block feed catch up failed");
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    async fn catch_up(&mut self) -> Result<()> {
        let info = self.client.consensus_info().await?;
        let tip = info.tip;
        let mut remaining_height = info.height;
        let mut stack = Vec::new();
        let mut cursor = tip;

        loop {
            if self.seen.contains(&cursor) {
                break;
            }

            if remaining_height == 0 {
                self.seen.insert(cursor);
                break;
            }

            let block = self
                .client
                .storage_block(&cursor)
                .await?
                .context("missing block while catching up")?;

            let parent = block.header().parent();
            stack.push((cursor, block));

            if self.seen.contains(&parent) || parent == cursor {
                break;
            }

            cursor = parent;
            remaining_height = remaining_height.saturating_sub(1);
        }

        let mut processed = 0usize;
        while let Some((header, block)) = stack.pop() {
            self.feed.ingest(header, block);
            self.seen.insert(header);
            processed += 1;
        }

        debug!(processed, "block feed processed catch up batch");
        Ok(())
    }
}

impl CleanupGuard for BlockFeedTask {
    fn cleanup(self: Box<Self>) {
        self.handle.abort();
    }
}

#[derive(Default)]
pub struct BlockStats {
    total_transactions: AtomicU64,
}

impl BlockStats {
    fn record_block(&self, block: &Block<SignedMantleTx>) {
        self.total_transactions
            .fetch_add(block.transactions().len() as u64, Ordering::Relaxed);
    }

    #[must_use]
    pub fn total_transactions(&self) -> u64 {
        self.total_transactions.load(Ordering::Relaxed)
    }
}
