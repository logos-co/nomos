use std::fmt::{Debug, Display};

use nomos_core::sdp::FinalizedBlockEvent;
use nomos_membership::{
    MembershipMessage, MembershipService, adapters::sdp::SdpAdapter, backends::MembershipBackend,
};
use overwatch::{DynError, overwatch::OverwatchHandle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct MembershipUpdateRequest {
    pub update_event: FinalizedBlockEvent,
}

pub async fn update_membership_handler<Backend, Sdp, StorageAdapter, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    payload: MembershipUpdateRequest,
) -> Result<(), DynError>
where
    Backend: MembershipBackend + Send + Sync + 'static,
    Sdp: SdpAdapter + Send + Sync + 'static,
    Backend::Settings: Clone,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<
            MembershipService<Backend, Sdp, StorageAdapter, RuntimeServiceId>,
        >,
{
    let relay = handle.relay().await?;

    let block_number = payload.update_event.block_number;

    relay
        .send(MembershipMessage::Update {
            block_number,
            update_event: payload.update_event,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(())
}
