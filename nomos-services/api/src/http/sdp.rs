use std::fmt::{Debug, Display};

use nomos_core::sdp::{ActivityMetadata, DeclarationId, DeclarationMessage};
use nomos_sdp::{SdpService, adapters::mempool::SdpMempoolAdapter as SdpMempoolAdapterTrait};
use overwatch::{DynError, overwatch::OverwatchHandle};

pub async fn post_declaration_handler<Backend, SdpMempoolAdapter, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    declaration: DeclarationMessage,
) -> Result<DeclarationId, DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    SdpMempoolAdapter: SdpMempoolAdapterTrait<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, SdpMempoolAdapter, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    relay
        .send(nomos_sdp::SdpMessage::PostDeclaration {
            declaration: Box::new(declaration),
            reply_channel: reply_tx,
        })
        .await
        .map_err(|(e, _)| e)?;

    reply_rx.await?
}

pub async fn post_activity_handler<Backend, SdpMempoolAdapter, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    metadata: ActivityMetadata,
) -> Result<(), DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    SdpMempoolAdapter: SdpMempoolAdapterTrait<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, SdpMempoolAdapter, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;

    relay
        .send(nomos_sdp::SdpMessage::PostActivity { metadata })
        .await
        .map_err(|(e, _)| e)?;

    Ok(())
}

pub async fn post_withdrawal_handler<Backend, SdpMempoolAdapter, RuntimeServiceId>(
    handle: OverwatchHandle<RuntimeServiceId>,
    declaration_id: DeclarationId,
) -> Result<(), DynError>
where
    Backend: nomos_sdp::backends::SdpBackend + Send + Sync + 'static,
    SdpMempoolAdapter: SdpMempoolAdapterTrait<RuntimeServiceId> + Send + Sync + 'static,
    RuntimeServiceId: Send
        + Sync
        + Debug
        + Display
        + 'static
        + overwatch::services::AsServiceId<SdpService<Backend, SdpMempoolAdapter, RuntimeServiceId>>,
{
    let relay = handle.relay().await?;

    relay
        .send(nomos_sdp::SdpMessage::PostWithdrawal { declaration_id })
        .await
        .map_err(|(e, _)| e)?;

    Ok(())
}
