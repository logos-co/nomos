use std::{fmt::Debug, marker::PhantomData};

use futures::Stream;
use kzgrs_backend::common::share::DaShare;
use libp2p::PeerId;
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    backends::libp2p::executor::{DaNetworkEvent, DaNetworkEventKind, DaNetworkExecutorBackend},
    membership::MembershipAdapter,
    NetworkService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipHandler;
use tokio_stream::StreamExt as _;

use crate::network::{adapters::common::adapter_for, NetworkAdapter};

adapter_for!(DaNetworkExecutorBackend, DaNetworkEventKind, DaNetworkEvent);
