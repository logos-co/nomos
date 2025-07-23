pub mod channel;

/// Tracks mantle ops
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct LedgerState { 
    channels: rpds::HashMap<ChannelId, ChannelState>,
}



impl LedgerState {
    pub fn try_apply_tx<Id, Constants: GasConstants>(
        mut self,
        tx: impl AuthenticatedMantleTx,
    ) -> Result<(Self, Value), LedgerError<Id>> {
        for op in &tx.mantle_tx().mantle_ops {
            match op {
                Op::Inscribe(_) => {
                    // Nothing to do :)
                }
                Op::
            }
        }

        Ok(self)
    }
}
