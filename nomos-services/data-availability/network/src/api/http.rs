use std::{fmt::Debug, net::IpAddr};

use async_trait::async_trait;
use common_http_client::CommonHttpClient;
use kzgrs_backend::common::share::{DaShare, DaSharesCommitments};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use nomos_core::da::BlobId;
use nomos_da_network_core::SubnetworkId;
use overwatch::DynError;
use rand::prelude::IteratorRandom as _;
use serde::{Deserialize, Serialize};
use subnetworks_assignations::MembershipHandler;
use tokio::sync::oneshot;
use tracing::error;
use url::Url;

use crate::api::ApiAdapter;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiAdapterSettings {
    pub api_port: u16,
    pub is_secure: bool,
}

#[derive(Clone)]
pub struct HttApiAdapter<Membership> {
    pub client: CommonHttpClient,
    pub membership: Membership,
    pub api_port: u16,
    pub protocol: String,
}

impl<Membership> HttApiAdapter<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
{
    fn random_peer_address(&self) -> Option<Url> {
        let peer_address = self
            .membership
            .members()
            .iter()
            .filter_map(|peer| self.membership.get_address(peer))
            .choose(&mut rand::thread_rng())?;

        let host = get_ip_from_multiaddr(&peer_address)?;
        match Url::parse(&format!("{}://{host}:{}", self.protocol, self.api_port)) {
            Ok(url) => Some(url),
            Err(e) => {
                error!("Failed to parse URL: {}", e);
                None
            }
        }
    }
}

#[async_trait]
impl<Membership> ApiAdapter for HttApiAdapter<Membership>
where
    Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
        + Clone
        + Debug
        + Send
        + Sync
        + 'static,
{
    type Settings = ApiAdapterSettings;
    type Share = DaShare;
    type BlobId = BlobId;
    type Commitments = DaSharesCommitments;
    type Membership = Membership;

    fn new(settings: Self::Settings, membership: Membership) -> Self {
        Self {
            client: CommonHttpClient::new(None),
            membership,
            api_port: settings.api_port,
            protocol: if settings.is_secure {
                "https".to_owned()
            } else {
                "http".to_owned()
            },
        }
    }

    async fn request_commitments(
        &self,
        blob_id: Self::BlobId,
        reply_channel: oneshot::Sender<Option<Self::Commitments>>,
    ) -> Result<(), DynError> {
        let Some(address) = self.random_peer_address() else {
            error!("No clients available");
            if reply_channel.send(None).is_err() {
                error!("Failed to send commitments reply");
            }
            return Ok(());
        };
        match self
            .client
            .get_commitments::<Self::Share>(address, blob_id)
            .await
        {
            Ok(commitments) => {
                if reply_channel.send(commitments).is_err() {
                    error!("Failed to send commitments reply");
                }
            }
            Err(e) => {
                error!("Failed to get commitments: {}", e);
                if reply_channel.send(None).is_err() {
                    error!("Failed to send commitments reply");
                }
            }
        }

        Ok(())
    }
}

fn get_ip_from_multiaddr(multiaddr: &Multiaddr) -> Option<IpAddr> {
    multiaddr.iter().find_map(|protocol| match protocol {
        multiaddr::Protocol::Ip4(ipv4) => Some(IpAddr::V4(ipv4)),
        multiaddr::Protocol::Ip6(ipv6) => Some(IpAddr::V6(ipv6)),
        _ => None,
    })
}
