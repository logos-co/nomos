use std::{collections::HashSet, fmt::Debug, pin::Pin};

use futures::{Stream, StreamExt as _};
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p_identity::PeerId;
use nomos_core::{da::BlobId, header::HeaderId, sdp::SessionNumber};
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    DaNetworkMsg, NetworkService,
    api::ApiAdapter as ApiAdapterTrait,
    backends::libp2p::{
        common::{HistoricSamplingEvent, SamplingEvent},
        validator::{
            DaNetworkEvent, DaNetworkEventKind, DaNetworkMessage, DaNetworkValidatorBackend,
        },
    },
    membership::{MembershipAdapter, handler::DaMembershipHandler},
    sdp::SdpAdapter as SdpAdapterTrait,
};
use overwatch::{
    DynError,
    services::{ServiceData, relay::OutboundRelay},
};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;

use crate::network::{CommitmentsEvent, NetworkAdapter, adapters::common::adapter_for};

adapter_for!(
    DaNetworkValidatorBackend,
    DaNetworkMessage,
    DaNetworkEventKind,
    DaNetworkEvent
);
