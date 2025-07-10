use std::{fmt::Debug, pin::Pin};

use futures::{Stream, StreamExt as _};
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p_identity::PeerId;
use nomos_core::da::BlobId;
use nomos_da_network_core::SubnetworkId;
use nomos_da_network_service::{
    api::ApiAdapter as ApiAdapterTrait,
    backends::libp2p::{
        common::SamplingEvent,
        validator::{
            DaNetworkEvent, DaNetworkEventKind, DaNetworkMessage, DaNetworkValidatorBackend,
        },
    },
    membership::{handler::DaMembershipHandler, MembershipAdapter},
    DaNetworkMsg, NetworkService,
};
use overwatch::{
    services::{relay::OutboundRelay, ServiceData},
    DynError,
};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;

use crate::network::{adapters::common::adapter_for, NetworkAdapter};

adapter_for!(
    DaNetworkValidatorBackend,
    DaNetworkMessage,
    DaNetworkEventKind,
    DaNetworkEvent
);
