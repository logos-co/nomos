// std
use std::fmt::Debug;
use std::marker::PhantomData;
// crates
use crate::network::adapters::common::adapter_for;
use futures::Stream;
use kzgrs_backend::common::blob::DaBlob;
use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::backends::libp2p::executor::{
    DaNetworkEvent, DaNetworkEventKind, DaNetworkExecutorBackend,
};
use nomos_da_network_service::NetworkService;
use overwatch_rs::services::relay::OutboundRelay;
use overwatch_rs::services::ServiceData;
use subnetworks_assignations::MembershipHandler;
use tokio_stream::StreamExt;
// internal
use crate::network::NetworkAdapter;

adapter_for!(DaNetworkExecutorBackend, DaNetworkEventKind, DaNetworkEvent);
