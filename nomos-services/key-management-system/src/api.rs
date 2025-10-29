use std::fmt::{Debug, Display};

use overwatch::{
    DynError,
    services::{AsServiceId, ServiceData, relay::OutboundRelay},
};
use tokio::sync::oneshot;

use crate::{
    KMSMessage, KMSOperatorKey, KMSService, KMSSigningStrategy, backend::KMSBackend,
    keys::secured_key::SecuredKey,
};

type KeyDescriptor<Backend> = (
    <Backend as KMSBackend>::KeyId,
    <<Backend as KMSBackend>::Key as SecuredKey>::PublicKey,
);

pub trait KmsServiceData: ServiceData<Message = KMSMessage<Self::Backend>> {
    type Backend: KMSBackend;
}

impl<B, RuntimeServiceId> KmsServiceData for KMSService<B, RuntimeServiceId>
where
    B: KMSBackend + 'static,
    B::KeyId: Debug,
    B::Key: Debug,
    B::Settings: Clone,
{
    type Backend = B;
}

pub struct KmsServiceApi<Kms, RuntimeServiceId>
where
    Kms: KmsServiceData,
{
    relay: OutboundRelay<Kms::Message>,
    _id: std::marker::PhantomData<RuntimeServiceId>,
}

impl<Kms, RuntimeServiceId> KmsServiceApi<Kms, RuntimeServiceId>
where
    Kms: KmsServiceData,
    <Kms::Backend as KMSBackend>::KeyId: Send,
    <Kms::Backend as KMSBackend>::Key: Send,
    <<Kms::Backend as KMSBackend>::Key as SecuredKey>::Payload: Send,
    <<Kms::Backend as KMSBackend>::Key as SecuredKey>::PublicKey: Send,
    <<Kms::Backend as KMSBackend>::Key as SecuredKey>::Signature: Send,
    RuntimeServiceId: AsServiceId<Kms> + Debug + Display + Sync,
{
    #[must_use]
    pub const fn new(relay: OutboundRelay<Kms::Message>) -> Self {
        Self {
            relay,
            _id: std::marker::PhantomData,
        }
    }

    pub async fn register(
        &self,
        key_id: <Kms::Backend as KMSBackend>::KeyId,
        key_type: <Kms::Backend as KMSBackend>::Key,
    ) -> Result<KeyDescriptor<Kms::Backend>, DynError> {
        let (reply_channel, rx) = oneshot::channel();

        self.relay
            .send(KMSMessage::Register {
                key_id,
                key_type,
                reply_channel,
            })
            .await
            .map_err(|_| "Failed to send register request")?;

        Ok(rx.await?)
    }

    pub async fn public_key(
        &self,
        key_id: <Kms::Backend as KMSBackend>::KeyId,
    ) -> Result<<<Kms::Backend as KMSBackend>::Key as SecuredKey>::PublicKey, DynError> {
        let (reply_channel, rx) = oneshot::channel();

        self.relay
            .send(KMSMessage::PublicKey {
                key_id,
                reply_channel,
            })
            .await
            .map_err(|_| "Failed to send public_key request")?;

        Ok(rx.await?)
    }

    pub async fn sign(
        &self,
        key_id: <Kms::Backend as KMSBackend>::KeyId,
        payload: <<Kms::Backend as KMSBackend>::Key as SecuredKey>::Payload,
    ) -> Result<<<Kms::Backend as KMSBackend>::Key as SecuredKey>::Signature, DynError> {
        let (reply_channel, rx) = oneshot::channel();

        self.relay
            .send(KMSMessage::Sign {
                signing_strategy: KMSSigningStrategy::Single(key_id),
                payload,
                reply_channel,
            })
            .await
            .map_err(|_| "Failed to send sign request")?;

        Ok(rx.await?)
    }

    pub async fn sign_multiple(
        &self,
        key_ids: Vec<<Kms::Backend as KMSBackend>::KeyId>,
        payload: <<Kms::Backend as KMSBackend>::Key as SecuredKey>::Payload,
    ) -> Result<<<Kms::Backend as KMSBackend>::Key as SecuredKey>::Signature, DynError> {
        let (reply_channel, rx) = oneshot::channel();

        self.relay
            .send(KMSMessage::Sign {
                signing_strategy: KMSSigningStrategy::Multi(key_ids),
                payload,
                reply_channel,
            })
            .await
            .map_err(|_| "Failed to send sign_multiple request")?;

        Ok(rx.await?)
    }

    pub async fn execute(
        &self,
        key_id: <Kms::Backend as KMSBackend>::KeyId,
        operator: KMSOperatorKey<
            <Kms::Backend as KMSBackend>::Key,
            <Kms::Backend as KMSBackend>::Error,
        >,
    ) -> Result<(), DynError> {
        self.relay
            .send(KMSMessage::Execute { key_id, operator })
            .await
            .map_err(|_| "Failed to send execute request")?;

        Ok(())
    }
}
