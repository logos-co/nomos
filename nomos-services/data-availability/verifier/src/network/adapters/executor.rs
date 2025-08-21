use std::{fmt::Debug, marker::PhantomData};

use futures::Stream;
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p::PeerId;
use nomos_core::{da::BlobId, mantle::SignedMantleTx};
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    api::ApiAdapter as ApiAdapterTrait,
    backends::libp2p::{
        common::VerificationEvent,
        executor::{DaNetworkEvent, DaNetworkEventKind, DaNetworkExecutorBackend},
    },
    membership::{handler::DaMembershipHandler, MembershipAdapter},
    NetworkService,
};
use overwatch::services::{relay::OutboundRelay, ServiceData};
use subnetworks_assignations::MembershipHandler;
use tokio_stream::StreamExt as _;

use crate::network::{adapters::common::adapter_for, NetworkAdapter, ValidationRequest};

adapter_for!(DaNetworkExecutorBackend, DaNetworkEventKind, DaNetworkEvent);
