use std::fmt::{Debug, Display};

use lb_binary_codec::bincode::{DeserializeOp, SerializeOp};
use lb_blend_service::message::{DataPayload, NetworkInfo, ProxyServiceMessage, ServiceMessage};
use lb_chain_service::{Epoch, api::CryptarchiaServiceApi};
use lb_core::{
    mantle::{NoteId, Value},
    sdp::{Declaration, DeclarationId, Locators, ProviderId, SNAPSHOT_FINALIZATION_DELAY},
};
use lb_key_management_system_keys::keys::ZkPublicKey;
use lb_ledger::mantle::sdp::rewards::blend::BlendRewardsInfo;
use lb_network_service::backends::libp2p::PeerId;
use overwatch::services::{AsServiceId, ServiceData};
use serde::Serialize;
use tokio::sync::oneshot;

use super::consensus::Cryptarchia;

/// Count of negotiated session peers for the current epoch, with how many are
/// healthy.
#[derive(Debug, Clone, Serialize)]
pub struct SessionPeers {
    pub total: usize,
    pub healthy: usize,
}

impl SessionPeers {
    /// Peer counts from the Blend network info: zeroed when the node is not a
    /// core node (no `core_info`).
    #[must_use]
    pub fn from_network_info<NodeId>(network_info: Option<&NetworkInfo<NodeId>>) -> Self {
        network_info
            .and_then(|info| info.core_info.as_ref())
            .map_or(
                Self {
                    total: 0,
                    healthy: 0,
                },
                |core_info| Self {
                    total: core_info.current_epoch_peers.len(),
                    healthy: core_info
                        .current_epoch_peers
                        .iter()
                        .filter(|(_, healthy)| *healthy)
                        .count(),
                },
            )
    }
}

/// A node's own Blend core declaration, as reported in [`BlendStatus`].
#[derive(Debug, Clone, Serialize)]
pub struct DeclarationInfo {
    pub id: DeclarationId,
    pub provider_id: ProviderId,
    pub zk_id: ZkPublicKey,
    pub locked_note_id: NoteId,
    pub locators: Locators,
    pub created_epoch: u32,
    /// The epoch the declaration's stake takes effect, derived as
    /// `created_epoch + SNAPSHOT_FINALIZATION_DELAY`.
    pub active_from_epoch: u32,
    pub withdraw_at: Option<u32>,
}

impl DeclarationInfo {
    #[must_use]
    pub fn from_declaration(id: DeclarationId, declaration: &Declaration) -> Self {
        let created_epoch = declaration.created.into_inner();
        Self {
            id,
            provider_id: declaration.provider_id,
            zk_id: declaration.zk_id,
            locked_note_id: declaration.service_note_id,
            locators: declaration.locators.clone(),
            created_epoch,
            active_from_epoch: created_epoch
                .saturating_add(SNAPSHOT_FINALIZATION_DELAY.into_inner()),
            withdraw_at: declaration.withdraw_at.map(Epoch::into_inner),
        }
    }
}

/// A snapshot of a node's Blend participation. Mirrors the referral program's
/// `blend_status()`, without the synthesized `state` and the `activity` block,
/// which need tracking the node does not keep yet.
#[derive(Debug, Clone, Serialize)]
pub struct BlendStatus {
    pub core_mode: bool,
    pub declaration: Option<DeclarationInfo>,
    pub current_epoch: u32,
    pub session_peers: SessionPeers,
}

/// This epoch's Blend reward estimate for a node, carrying its reward key
/// alongside the ledger-provided reward fields in one flat object.
#[derive(Debug, Clone, Serialize)]
pub struct BlendRewardsResponse {
    pub reward_key: ZkPublicKey,
    #[serde(flatten)]
    pub rewards: BlendRewardsInfo,
}

pub async fn blend_info<BlendService, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<Option<NetworkInfo<PeerId>>, overwatch::DynError>
where
    BlendService: ServiceData<Message = ProxyServiceMessage<ServiceMessage<PeerId>>>,
    RuntimeServiceId: AsServiceId<BlendService> + Debug + Sync + Display + 'static,
{
    let relay = handle.relay::<BlendService>().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(ServiceMessage::GetNetworkInfo { reply: sender }.into())
        .await
        .map_err(|error| overwatch::DynError::from(error.to_string()))?;

    receiver
        .await
        .map_err(|e| Box::new(e) as overwatch::DynError)
}

/// The Blend reward state at the tip for the node identified by `reward_key`,
/// or `None` when no Blend service is registered on the chain.
pub async fn blend_rewards<RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    reward_key: ZkPublicKey,
) -> Result<Option<BlendRewardsInfo>, overwatch::DynError>
where
    RuntimeServiceId:
        Debug + Send + Sync + Display + 'static + AsServiceId<Cryptarchia<RuntimeServiceId>>,
{
    let chain_api =
        CryptarchiaServiceApi::<Cryptarchia<RuntimeServiceId>>::from_overwatch_handle(handle).await;
    Ok(chain_api.get_blend_rewards(reward_key).await?)
}

pub async fn blend_zk_id<BlendService, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<ZkPublicKey, overwatch::DynError>
where
    BlendService: ServiceData<Message = ProxyServiceMessage<ServiceMessage<PeerId>>>,
    RuntimeServiceId: AsServiceId<BlendService> + Debug + Sync + Display + 'static,
{
    let relay = handle.relay::<BlendService>().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(ProxyServiceMessage::GetZkId { reply: sender })
        .await
        .map_err(|error| overwatch::DynError::from(error.to_string()))?;

    receiver
        .await
        .map_err(|e| Box::new(e) as overwatch::DynError)
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServiceNoteError {
    #[error("holds no notes")]
    NoNotes,
    #[error("all {locked} notes are already locked for blend")]
    AllLocked { locked: usize },
    #[error(
        "no single note meets min stake {threshold}: largest is {largest}, total {total}; consolidation may be possible"
    )]
    NoSingleNoteMeetsThreshold {
        largest: Value,
        total: Value,
        threshold: Value,
    },
    #[error("insufficient total collateral: {total} < min stake {threshold}")]
    InsufficientTotal { total: Value, threshold: Value },
}

/// Picks the smallest unlocked note that meets `threshold`, so the least
/// collateral is locked and larger notes stay free.
pub fn select_service_note<Id: Copy>(
    notes: &[(Id, Value)],
    is_locked: impl Fn(Id) -> bool,
    threshold: Value,
) -> Result<Id, ServiceNoteError> {
    if notes.is_empty() {
        return Err(ServiceNoteError::NoNotes);
    }

    let unlocked: Vec<_> = notes.iter().filter(|(id, _)| !is_locked(*id)).collect();
    if unlocked.is_empty() {
        return Err(ServiceNoteError::AllLocked {
            locked: notes.len(),
        });
    }

    if let Some((id, _)) = unlocked
        .iter()
        .filter(|(_, value)| *value >= threshold)
        .min_by_key(|(_, value)| *value)
    {
        return Ok(*id);
    }

    let total = unlocked.iter().map(|(_, value)| *value).sum();
    if total >= threshold {
        let largest = unlocked.iter().map(|(_, value)| *value).max().unwrap_or(0);
        return Err(ServiceNoteError::NoSingleNoteMeetsThreshold {
            largest,
            total,
            threshold,
        });
    }

    Err(ServiceNoteError::InsufficientTotal { total, threshold })
}

pub async fn blend_join_network<BlendService, RuntimeServiceId>(
    handle: &overwatch::overwatch::OverwatchHandle<RuntimeServiceId>,
    locator: lb_core::sdp::Locator,
    service_note_id: NoteId,
) -> Result<lb_sdp_service::SdpSubmission, overwatch::DynError>
where
    BlendService: ServiceData<Message = ProxyServiceMessage<ServiceMessage<PeerId>>>,
    RuntimeServiceId: AsServiceId<BlendService> + Debug + Sync + Display + 'static,
{
    let relay = handle.relay::<BlendService>().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(ProxyServiceMessage::JoinAsCore {
            locator,
            service_note_id,
            reply: sender,
        })
        .await
        .map_err(|error| overwatch::DynError::from(error.to_string()))?;

    let result = receiver
        .await
        .map_err(|e| Box::new(e) as overwatch::DynError)??;

    Ok(result)
}

/// Sends a transaction through the Blend network without adding it to this
/// node's own mempool.
///
/// Returns the transaction's id. Getting one back means the transaction was
/// accepted for blending, not that it was sent.
pub async fn blend_transaction<BlendService, Transaction, Id, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    transaction: Transaction,
    id: impl Fn(&Transaction) -> Id,
) -> Result<Id, overwatch::DynError>
where
    BlendService: ServiceData<Message = ProxyServiceMessage<ServiceMessage<PeerId>>>,
    Transaction: SerializeOp,
    RuntimeServiceId: AsServiceId<BlendService> + Debug + Sync + Display + 'static,
{
    // Encoded the same way the mempool gossips transactions, so that whichever
    // node exits this one decodes what it expects.
    let payload = DataPayload::try_from_transaction(&transaction)?;
    let relay = handle.relay::<BlendService>().await?;

    relay
        .send(ServiceMessage::Blend(payload).into())
        .await
        .map_err(|error| overwatch::DynError::from(error.to_string()))?;

    Ok(id(&transaction))
}

/// The ids of the transactions this node is still waiting on a `PoW` solution
/// for.
///
/// Only those: once a transaction has been encapsulated it is in the
/// scheduler's hands, waiting for a release round, and no longer reported here.
// TODO: Have Blend hand back transactions rather than bytes, so that this can
// report their ids without decoding them again.
pub async fn blend_pending_transactions<BlendService, Transaction, Id, RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
    id: impl Fn(&Transaction) -> Id,
) -> Result<Vec<Id>, overwatch::DynError>
where
    BlendService: ServiceData<Message = ProxyServiceMessage<ServiceMessage<PeerId>>>,
    Transaction: DeserializeOp,
    RuntimeServiceId: AsServiceId<BlendService> + Debug + Sync + Display + 'static,
{
    let relay = handle.relay::<BlendService>().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(ServiceMessage::GetPendingTransactions { reply: sender }.into())
        .await
        .map_err(|error| overwatch::DynError::from(error.to_string()))?;

    receiver
        .await
        .map_err(|e| Box::new(e) as overwatch::DynError)?
        .iter()
        .map(|encoded_transaction| {
            // These are transactions this node encoded itself on the way in, so
            // a decoding failure here is this node disagreeing with itself.
            Ok(id(&Transaction::from_bytes(encoded_transaction)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never_locked(_: u8) -> bool {
        false
    }

    #[test]
    fn picks_smallest_note_that_meets_threshold() {
        let notes = [(1u8, 1000), (2, 3000), (3, 10000), (4, 5050)];
        assert_eq!(select_service_note(&notes, never_locked, 5000), Ok(4));
    }

    #[test]
    fn exact_threshold_qualifies() {
        let notes = [(1u8, 5000)];
        assert_eq!(select_service_note(&notes, never_locked, 5000), Ok(1));
    }

    #[test]
    fn total_sufficient_but_no_single_note() {
        let notes = [(1u8, 1000), (2, 2000), (3, 3000)];
        assert_eq!(
            select_service_note(&notes, never_locked, 5000),
            Err(ServiceNoteError::NoSingleNoteMeetsThreshold {
                largest: 3000,
                total: 6000,
                threshold: 5000,
            })
        );
    }

    #[test]
    fn total_insufficient() {
        let notes = [(1u8, 1000), (2, 2000)];
        assert_eq!(
            select_service_note(&notes, never_locked, 5000),
            Err(ServiceNoteError::InsufficientTotal {
                total: 3000,
                threshold: 5000,
            })
        );
    }

    #[test]
    fn locked_notes_are_skipped() {
        let notes = [(1u8, 10000), (2, 6000)];
        assert_eq!(select_service_note(&notes, |id| id == 1, 5000), Ok(2));
    }

    #[test]
    fn all_locked() {
        let notes = [(1u8, 10000)];
        assert_eq!(
            select_service_note(&notes, |_| true, 5000),
            Err(ServiceNoteError::AllLocked { locked: 1 })
        );
    }

    #[test]
    fn no_notes() {
        let notes: [(u8, Value); 0] = [];
        assert_eq!(
            select_service_note(&notes, never_locked, 5000),
            Err(ServiceNoteError::NoNotes)
        );
    }
}
