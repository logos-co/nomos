use std::fmt::Display;

use async_trait::async_trait;
use backends::MembershipBackend;
use futures::StreamExt;
use overwatch::{
    services::{
        state::{NoOperator, NoState},
        AsServiceId, ServiceCore, ServiceData,
    },
    OpaqueServiceStateHandle,
};
use services_utils::overwatch::lifecycle;

pub mod backends;

#[derive(Debug, Clone)]
pub enum MembershipMessage {
    // todo: define API
}

pub struct MembershipService<B, RuntimeServiceId>
where
    B: MembershipBackend,
{
    backend: B,
    service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
}

impl<B, RuntimeServiceId> ServiceData for MembershipService<B, RuntimeServiceId>
where
    B: MembershipBackend,
{
    type Settings = ();
    type State = NoState<Self::Settings>;
    type StateOperator = NoOperator<Self::State>;
    type Message = MembershipMessage;
}

#[async_trait]
impl<B, RuntimeServiceId> ServiceCore<RuntimeServiceId> for MembershipService<B, RuntimeServiceId>
where
    B: MembershipBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    fn init(
        service_state: OpaqueServiceStateHandle<Self, RuntimeServiceId>,
        _initstate: Self::State,
    ) -> Result<Self, overwatch::DynError> {
        Ok(Self {
            backend: B::init(
                // todo:  
            ),
            service_state,
        })
    }

    async fn run(mut self) -> Result<(), overwatch::DynError> {
        let mut lifecycle_stream = self.service_state.lifecycle_handle.message_stream();
        loop {
            tokio::select! {
                Some(msg) = self.service_state.inbound_relay.recv()  => {
                    self.handle_membership_message(msg).await;
                }
                Some(msg) = lifecycle_stream.next() => {
                    if lifecycle::should_stop_service::<Self, RuntimeServiceId>(&msg) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<B, RuntimeServiceId> MembershipService<B, RuntimeServiceId>
where
    B: MembershipBackend + Send + Sync + 'static,
    RuntimeServiceId: AsServiceId<Self> + Clone + Display + Send + Sync + 'static,
{
    async fn handle_membership_message(&mut self, msg: MembershipMessage) {
        todo!()
    }
}
