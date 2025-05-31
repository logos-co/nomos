use async_trait::async_trait;
use nomos_membership::{backends::MembershipBackendError, MembershipSnapshotStream};
use nomos_sdp_core::ServiceType;
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use thiserror::Error;

pub mod membership_service;

#[derive(Debug, Error)]
pub enum MembershipAdapterError {
    Backend(MembershipBackendError),
    Other(DynError),
}

impl std::fmt::Display for MembershipAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(err) => write!(f, "Backend error: {err:?}"),
            Self::Other(err) => write!(f, "Other error: {err}"),
        }
    }
}

#[async_trait]
pub trait MembershipAdapter {
    type MembershipService: ServiceData;

    fn new(relay: OutboundRelay<<Self::MembershipService as ServiceData>::Message>) -> Self;

    async fn subscribe(
        &self,
        service_type: ServiceType,
    ) -> Result<MembershipSnapshotStream, MembershipAdapterError>;
}
