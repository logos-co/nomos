use chain_service::api::{CryptarchiaServiceApi, CryptarchiaServiceData};
use fork_stream::Forked;
use futures::{StreamExt as _, stream::BoxStream};
use nomos_blend_scheduling::session::SessionEvent;
use nomos_time::SlotTick;
use overwatch::overwatch::OverwatchHandle;

use crate::{
    epoch_info::{EpochEvent, EpochHandler, LeaderInputsMinusQuota, PolEpochInfo},
    membership::MembershipInfo,
    message::ServiceMessage,
};

/// Holds all the streams used across different [`Mode`]s.
///
/// It can be passed to a mode and handled over to the next mode.
/// If the first mode have read all the streams by [`Context::advance`] without
/// delay, the next mode will be able to immediately read the latest item from
/// each stream.
pub struct Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>
where
    NodeId: Clone,
    ChainService: CryptarchiaServiceData,
{
    pub service_message_stream: BoxStream<'static, ServiceMessage<BroadcastSettings>>,

    pub session_stream: Forked<BoxStream<'static, SessionEvent<MembershipInfo<NodeId>>>>,
    pub current_membership_info: MembershipInfo<NodeId>,

    pub epoch_handler:
        EpochHandler<CryptarchiaServiceApi<ChainService, RuntimeServiceId>, RuntimeServiceId>,
    pub current_leader_inputs_minus_quota: LeaderInputsMinusQuota,

    pub clock_stream: BoxStream<'static, SlotTick>,
    pub current_clock: SlotTick,

    pub secret_pol_info_stream: BoxStream<'static, PolEpochInfo>,
    pub current_private_leader_info: PolEpochInfo,

    pub overwatch_handle: OverwatchHandle<RuntimeServiceId>,
}

impl<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>
    Context<NodeId, BroadcastSettings, ChainService, RuntimeServiceId>
where
    NodeId: Clone,
    ChainService: CryptarchiaServiceData<Tx: Send + Sync>,
    RuntimeServiceId: Send + Sync,
{
    /// Advances the context by waiting for the next item from any of the
    /// streams.
    ///
    /// It also updates the current state (e.g. self.current_*) based on the
    /// received item.
    pub async fn advance(&mut self) -> Option<Event<NodeId, BroadcastSettings>> {
        tokio::select! {
            Some(message) = self.service_message_stream.next() => {
                Some(Event::ServiceMessage(message))
            }
            Some(clock_tick) = self.clock_stream.next() => {
                self.current_clock = clock_tick;

                let maybe_epoch_event = self.epoch_handler.tick(clock_tick).await;
                if let Some(EpochEvent::NewEpoch(leader_inputs_minus_quota) | EpochEvent::NewEpochAndOldEpochTransitionExpired(leader_inputs_minus_quota)) = maybe_epoch_event {
                    self.current_leader_inputs_minus_quota = leader_inputs_minus_quota;
                }

                Some(Event::ClockTick {
                    clock_tick,
                    maybe_epoch_event,
                })
            }
            Some(pol_info) = self.secret_pol_info_stream.next() => {
                self.current_private_leader_info = pol_info.clone();
                Some(Event::SecretPolInfo(pol_info))
            }
            Some(session_event) = self.session_stream.next() => {
                if let SessionEvent::NewSession(membership_info) = &session_event {
                    self.current_membership_info = membership_info.clone();
                }
                Some(Event::SessionEvent(session_event))
            }
            else => {
                // All streams have ended
                None
            }
        }
    }
}

/// Items received from the streams that [`Context`] holds.
#[expect(
    clippy::large_enum_variant,
    reason = "necessary for various event types"
)]
pub enum Event<NodeId, BroadcastSettings> {
    ServiceMessage(ServiceMessage<BroadcastSettings>),
    ClockTick {
        clock_tick: SlotTick,
        maybe_epoch_event: Option<EpochEvent>,
    },
    SecretPolInfo(PolEpochInfo),
    SessionEvent(SessionEvent<MembershipInfo<NodeId>>),
}
