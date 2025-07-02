use std::fmt::{Debug, Display};

use nomos_membership::{
    adapters::SdpAdapter, backends::MembershipBackend, MembershipMessage, MembershipService,
};
use nomos_sdp_core::FinalizedBlockEvent;
use overwatch::{overwatch::OverwatchHandle, DynError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct MembershipUpdateRequest {
    pub update_event: FinalizedBlockEvent,
}

pub async fn update_membership_handler<Backend, Sdp, RuntimeServiceId>(
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
        + overwatch::services::AsServiceId<MembershipService<Backend, Sdp, RuntimeServiceId>>,
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
