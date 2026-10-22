//! Wall-clock position of a slot, for measuring how long after its slot
//! started a block is seen by this node.

use std::time::{SystemTime, UNIX_EPOCH};

use lb_cryptarchia_engine::Slot;
use overwatch::{DynError, services::relay::OutboundRelay};
use tokio::sync::oneshot;

use crate::{TimeServiceInfo, TimeServiceMessage};

/// Maps slots to wall-clock time using the chain's genesis time and slot
/// duration.
#[derive(Clone, Copy, Debug)]
pub struct SlotClock {
    genesis_time_unix_ms: i64,
    slot_duration_ms: u64,
}

impl SlotClock {
    #[must_use]
    pub const fn new(genesis_time_unix_ms: i64, slot_duration_ms: u64) -> Self {
        Self {
            genesis_time_unix_ms,
            slot_duration_ms,
        }
    }

    #[must_use]
    pub const fn from_info(info: &TimeServiceInfo) -> Self {
        Self::new(info.genesis_time_unix_ms, info.slot_duration_ms)
    }

    /// Asks the time service for the chain's slot configuration.
    ///
    /// # Errors
    ///
    /// If the time service cannot be reached or refuses to answer.
    pub async fn fetch(relay: &OutboundRelay<TimeServiceMessage>) -> Result<Self, DynError> {
        let (sender, receiver) = oneshot::channel();
        relay
            .send(TimeServiceMessage::Info { sender })
            .await
            .map_err(|error| DynError::from(format!("failed to request time info: {error}")))?;
        let info = receiver.await?.map_err(DynError::from)?;
        Ok(Self::from_info(&info))
    }

    /// Unix time, in milliseconds, at which `slot` starts.
    #[must_use]
    pub fn slot_start_unix_ms(&self, slot: Slot) -> i64 {
        let offset = i64::try_from(u64::from(slot).saturating_mul(self.slot_duration_ms))
            .unwrap_or(i64::MAX);
        self.genesis_time_unix_ms.saturating_add(offset)
    }

    /// Milliseconds elapsed since `slot` started. Negative when the slot has
    /// not started yet according to this node's clock.
    #[must_use]
    pub fn millis_since_slot_start(&self, slot: Slot) -> i64 {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        now_ms.saturating_sub(self.slot_start_unix_ms(slot))
    }
}
