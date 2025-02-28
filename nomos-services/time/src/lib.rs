use std::{
    fmt::{Debug, Formatter},
    pin::Pin,
};

use cryptarchia_engine::{Epoch, Slot};
use futures::{Stream, StreamExt};
use log::error;
use overwatch_rs::{
    services::{
        relay::RelayMessage,
        state::{NoOperator, NoState},
        ServiceCore, ServiceData, ServiceId,
    },
    DynError, OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle::should_stop_service;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use crate::backends::TimeBackend;

pub mod backends;

const TIME_SERVICE_TAG: ServiceId = "time-service";

#[derive(Clone, Debug)]
pub struct SlotTick {
    pub epoch: Epoch,
    pub slot: Slot,
}

pub type EpochSlotTickStream = Pin<Box<dyn Stream<Item = SlotTick> + Send + Sync + Unpin>>;

pub enum TimeServiceMessage {
    Subscribe {
        sender: oneshot::Sender<EpochSlotTickStream>,
    },
}

impl Debug for TimeServiceMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Subscribe { .. } => f.write_str("New time service subscription"),
        }
    }
}

impl RelayMessage for TimeServiceMessage {}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct TimeServiceSettings<BackendSettings> {
    pub backend_settings: BackendSettings,
}

pub struct TimeService<Backend>
where
    Backend: TimeBackend,
    Backend::Settings: Clone,
{
    state: OpaqueServiceStateHandle<Self>,
    backend: Backend,
}

impl<Backend> ServiceData for TimeService<Backend>
where
    Backend: TimeBackend,
    Backend::Settings: Clone,
{
    const SERVICE_ID: ServiceId = TIME_SERVICE_TAG;
    type Settings = TimeServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State, Self::Settings>;
    type Message = TimeServiceMessage;
}

#[async_trait::async_trait]
impl<Backend> ServiceCore for TimeService<Backend>
where
    Backend: TimeBackend + Send,
    Backend::Settings: Clone + Send + Sync,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let Self::Settings {
            backend_settings, ..
        } = service_state.settings_reader.get_updated_settings();
        let backend = Backend::init(backend_settings);
        Ok(Self {
            state: service_state,
            backend,
        })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self { state, backend } = self;
        let mut inbound_relay = state.inbound_relay;
        let mut lifecycle_relay = state.lifecycle_handle.message_stream();
        let mut tick_stream = backend.tick_stream();

        // 3 slots buffer should be enough
        const SLOTS_BUFFER: usize = 3;
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(SLOTS_BUFFER);

        loop {
            tokio::select! {
                Some(service_message) = inbound_relay.recv() => {
                    match service_message {
                        TimeServiceMessage::Subscribe { sender} => {
                            let channel_stream = BroadcastStream::new(broadcast_receiver.resubscribe()).filter_map(|r| Box::pin(async {match r {
                                Ok(tick) => Some(tick),
                                Err(e) => {
                                    // log lagging errors, services should always aim to be ready for next slot
                                    error!("Lagging behind slot ticks: {e:?}");
                                    None
                                }
                            }}));
                            let stream = Pin::new(Box::new(channel_stream));
                            if let Err(_e) = sender.send(stream) {
                                error!("Error subscribing to time event: Couldn't send back a response");
                            };
                        }
                    }
                }
                Some(slot_tick) = tick_stream.next() => {
                    if let Err(e) = broadcast_sender.send(slot_tick) {
                        error!("Error updating slot tick: {e}");
                    }

                }
                Some(lifecycle_msg) = lifecycle_relay.next() => {
                    if should_stop_service::<Self>(&lifecycle_msg).await {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
