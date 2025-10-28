use std::fmt::{Debug, Display};

use nomos_core::sdp::{ActivityMetadata, DeclarationId, DeclarationMessage};
use nomos_sdp::SdpService;
use overwatch::{DynError, overwatch::OverwatchHandle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DeclarationRequest {
    pub declaration: DeclarationMessage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivityRequest {
    pub metadata: ActivityMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WithdrawalRequest {
    pub declaration_id: DeclarationId,
}

pub async fn post_declaration_handler<Backend, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    payload: DeclarationRequest,
) -> Result<DeclarationId, DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    relay
        .send(nomos_sdp::SdpMessage::PostDeclaration {
            declaration: Box::new(payload.declaration),
            reply_channel: reply_tx,
        })
        .await
        .map_err(|(e, _)| e)?;

    reply_rx.await?
}

pub async fn post_activity_handler<Backend, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    payload: ActivityRequest,
) -> Result<(), DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;

    relay
        .send(nomos_sdp::SdpMessage::PostActivity {
            metadata: payload.metadata,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(())
}

pub async fn post_withdrawal_handler<Backend, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    payload: WithdrawalRequest,
) -> Result<(), DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;

    relay
        .send(nomos_sdp::SdpMessage::PostWithdrawal {
            declaration_id: payload.declaration_id,
        })
        .await
        .map_err(|(e, _)| e)?;

    Ok(())
}
