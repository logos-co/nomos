#![allow(
    clippy::disallowed_script_idents,
    reason = "The crate `cfg_eval` contains Sinhala script identifiers. \
    Using the `expect` or `allow` macro on top of their usage does not remove the warning"
)]

use std::{
    fmt::{Debug, Display, Formatter},
    pin::Pin,
};

use cryptarchia_engine::{Epoch, Slot};
use futures::{Stream, StreamExt as _};
use log::error;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

use crate::backends::TimeBackend;

pub mod backends;

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

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug)]
pub struct TimeServiceSettings<BackendSettings> {
    pub backend_settings: BackendSettings,
}

pub struct TimeService<Backend, RuntimeServiceId>
where
    Backend: TimeBackend,
    Backend::Settings: Clone,
{
    service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
    backend: Backend,
}

impl<Backend, RuntimeServiceId> ServiceData for TimeService<Backend, RuntimeServiceId>
where
    Backend: TimeBackend,
    Backend::Settings: Clone,
{
    type Settings = TimeServiceSettings<Backend::Settings>;
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = TimeServiceMessage;
}

#[async_trait::async_trait]
impl<Backend, RuntimeServiceId> ServiceCore<RuntimeServiceId>
    for TimeService<Backend, RuntimeServiceId>
where
    Backend: TimeBackend + Send,
    Backend::Settings: Clone + Send + Sync,
    RuntimeServiceId: AsServiceId<Self> + Display + Send,
{
    fn init(
        service_resources_handle: OpaqueServiceResourcesHandle<Self, RuntimeServiceId>,
        _initial_state: Self::State,
    ) -> Result<Self, DynError> {
        let Self::Settings {
            backend_settings, ..
        } = service_resources_handle
            .settings_handle
            .notifier()
            .get_updated_settings();
        let backend = Backend::init(backend_settings);
        Ok(Self {
            service_resources_handle,
            backend,
        })
    }

    async fn run(self) -> Result<(), DynError> {
        let Self {
            service_resources_handle,
            backend,
        } = self;
        let mut inbound_relay = service_resources_handle.inbound_relay;
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
            }
        }
    }
}
