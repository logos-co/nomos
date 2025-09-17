use crate::{ConsensusMsg, CryptarchiaInfo};
use nomos_core::header::HeaderId;
use overwatch::{
    services::{relay::OutboundRelay, AsServiceId, ServiceData},
    DynError, OpaqueServiceResourcesHandle,
};
use tokio::sync::{broadcast, oneshot};

pub trait CryptarchiaServiceData: ServiceData<Message = ConsensusMsg> + Send + 'static {}
impl<T> CryptarchiaServiceData for T where T: ServiceData<Message = ConsensusMsg> + Send + 'static {}

pub struct CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
{
    relay: OutboundRelay<Cryptarchia::Message>,
    _id: std::marker::PhantomData<RuntimeServiceId>,
}

impl<Cryptarchia, RuntimeServiceId> CryptarchiaServiceApi<Cryptarchia, RuntimeServiceId>
where
    Cryptarchia: CryptarchiaServiceData,
    RuntimeServiceId: AsServiceId<Cryptarchia> + std::fmt::Debug + std::fmt::Display + Sync,
{
    /// Create a new API instance
    pub async fn new<S>(
        service_resources_handle: &OpaqueServiceResourcesHandle<S, RuntimeServiceId>,
    ) -> Result<Self, DynError>
    where
        S: ServiceData,
        S::Message: Send + Sync,
        S::State: Send + Sync,
        S::Settings: Send + Sync,
    {
        let relay = service_resources_handle
            .overwatch_handle
            .relay::<Cryptarchia>()
            .await?;

        Ok(Self {
            relay,
            _id: std::marker::PhantomData,
        })
    }

    /// Get the current consensus info including LIB, tip, slot, height, and mode
    pub async fn info(&self) -> Result<CryptarchiaInfo, String> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::Info { tx })
            .await
            .map_err(|_| "Failed to send info request")?;

        rx.await
            .map_err(|e| format!("Failed to receive info response: {e:?}"))
    }

    /// Subscribe to new blocks
    pub async fn subscribe_new_blocks(&self) -> Result<broadcast::Receiver<HeaderId>, String> {
        let (sender, receiver) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::NewBlockSubscribe { sender })
            .await
            .map_err(|_| "Failed to send block subscription request")?;

        receiver
            .await
            .map_err(|e| format!("Failed to receive block subscription: {e:?}"))
    }

    /// Get headers in the range from `from` to `to`
    /// If `from` is None, defaults to tip
    /// If `to` is None, defaults to LIB
    pub async fn get_headers(
        &self,
        from: Option<HeaderId>,
        to: Option<HeaderId>,
    ) -> Result<Vec<HeaderId>, String> {
        let (tx, rx) = oneshot::channel();

        self.relay
            .send(ConsensusMsg::GetHeaders { from, to, tx })
            .await
            .map_err(|_| "Failed to send headers request")?;

        rx.await
            .map_err(|e| format!("Failed to receive headers response: {e:?}"))
    }

    /// Get all headers from a specific block to LIB
    pub async fn get_headers_to_lib(&self, from: HeaderId) -> Result<Vec<HeaderId>, String> {
        self.get_headers(Some(from), None).await
    }

    /// Get all headers from tip to a specific block
    pub async fn get_headers_from_tip(&self, to: HeaderId) -> Result<Vec<HeaderId>, String> {
        self.get_headers(None, Some(to)).await
    }
}
