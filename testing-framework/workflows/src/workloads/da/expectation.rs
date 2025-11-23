use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use nomos_core::mantle::{
    AuthenticatedMantleTx as _,
    ops::{Op, channel::ChannelId},
};
use testing_framework_core::scenario::{BlockRecord, DynError, Expectation, RunContext};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug)]
pub struct DaWorkloadExpectation {
    planned_channels: Vec<ChannelId>,
    capture_state: Option<CaptureState>,
}

#[derive(Debug)]
struct CaptureState {
    planned: Arc<HashSet<ChannelId>>,
    inscriptions: Arc<Mutex<HashSet<ChannelId>>>,
    blobs: Arc<Mutex<HashSet<ChannelId>>>,
}

const MIN_INCLUSION_RATIO: f64 = 0.8;

#[derive(Debug, Error)]
enum DaExpectationError {
    #[error("da workload expectation not started")]
    NotCaptured,
    #[error("missing inscriptions for {missing:?}")]
    MissingInscriptions { missing: Vec<ChannelId> },
    #[error("missing blobs for {missing:?}")]
    MissingBlobs { missing: Vec<ChannelId> },
}

impl DaWorkloadExpectation {
    pub const fn new(planned_channels: Vec<ChannelId>) -> Self {
        Self {
            planned_channels,
            capture_state: None,
        }
    }
}

#[async_trait]
impl Expectation for DaWorkloadExpectation {
    fn name(&self) -> &'static str {
        "da_workload_inclusions"
    }

    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        if self.capture_state.is_some() {
            return Ok(());
        }

        let planned = Arc::new(
            self.planned_channels
                .iter()
                .copied()
                .collect::<HashSet<_>>(),
        );
        let inscriptions = Arc::new(Mutex::new(HashSet::new()));
        let blobs = Arc::new(Mutex::new(HashSet::new()));

        let mut receiver = ctx.block_feed().subscribe();
        let planned_for_task = Arc::clone(&planned);
        let inscriptions_for_task = Arc::clone(&inscriptions);
        let blobs_for_task = Arc::clone(&blobs);

        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(record) => capture_block(
                        record.as_ref(),
                        &planned_for_task,
                        &inscriptions_for_task,
                        &blobs_for_task,
                    ),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        self.capture_state = Some(CaptureState {
            planned,
            inscriptions,
            blobs,
        });

        Ok(())
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        let state = self
            .capture_state
            .as_ref()
            .ok_or(DaExpectationError::NotCaptured)
            .map_err(DynError::from)?;

        let planned_total = state.planned.len();
        let missing_inscriptions = {
            let inscriptions = state
                .inscriptions
                .lock()
                .expect("inscription lock poisoned");
            missing_channels(&state.planned, &inscriptions)
        };
        let required_inscriptions = minimum_required(planned_total, MIN_INCLUSION_RATIO);
        if planned_total.saturating_sub(missing_inscriptions.len()) < required_inscriptions {
            return Err(DaExpectationError::MissingInscriptions {
                missing: missing_inscriptions,
            }
            .into());
        }

        let missing_blobs = {
            let blobs = state.blobs.lock().expect("blob lock poisoned");
            missing_channels(&state.planned, &blobs)
        };
        let required_blobs = minimum_required(planned_total, MIN_INCLUSION_RATIO);
        if planned_total.saturating_sub(missing_blobs.len()) < required_blobs {
            return Err(DaExpectationError::MissingBlobs {
                missing: missing_blobs,
            }
            .into());
        }

        Ok(())
    }
}

fn capture_block(
    block: &BlockRecord,
    planned: &HashSet<ChannelId>,
    inscriptions: &Arc<Mutex<HashSet<ChannelId>>>,
    blobs: &Arc<Mutex<HashSet<ChannelId>>>,
) {
    let mut new_inscriptions = Vec::new();
    let mut new_blobs = Vec::new();

    for tx in block.block.transactions() {
        for op in &tx.mantle_tx().ops {
            match op {
                Op::ChannelInscribe(inscribe) if planned.contains(&inscribe.channel_id) => {
                    new_inscriptions.push(inscribe.channel_id);
                }
                Op::ChannelBlob(blob) if planned.contains(&blob.channel) => {
                    new_blobs.push(blob.channel);
                }
                _ => {}
            }
        }
    }

    if !new_inscriptions.is_empty() {
        let mut guard = inscriptions.lock().expect("inscription lock poisoned");
        guard.extend(new_inscriptions);
    }

    if !new_blobs.is_empty() {
        let mut guard = blobs.lock().expect("blob lock poisoned");
        guard.extend(new_blobs);
    }
}

fn missing_channels(planned: &HashSet<ChannelId>, observed: &HashSet<ChannelId>) -> Vec<ChannelId> {
    planned.difference(observed).copied().collect()
}

fn minimum_required(total: usize, ratio: f64) -> usize {
    ((total as f64) * ratio).ceil() as usize
}
