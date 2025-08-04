macro_rules! adapter_for {
    ($DaNetworkBackend:ident, $DaNetworksEventKind:ident, $DaNetworkEvent:ident) => {
        pub struct Libp2pAdapter<
            Membership,
            MembershipServiceAdapter,
            StorageAdapter,
            ApiAdapter,
            RuntimeServiceId,
        > where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Clone
                + Debug
                + Send
                + Sync
                + 'static,
            MembershipServiceAdapter: MembershipAdapter,
            ApiAdapter: ApiAdapterTrait,
        {
            network_relay: OutboundRelay<
                <NetworkService<
                    $DaNetworkBackend<Membership>,
                    Membership,
                    MembershipServiceAdapter,
                    StorageAdapter,
                    ApiAdapter,
                    RuntimeServiceId,
                > as ServiceData>::Message,
            >,
            _membership: PhantomData<Membership>,
        }

        #[async_trait::async_trait]
        impl<
                Membership,
                MembershipServiceAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            > NetworkAdapter<RuntimeServiceId>
            for Libp2pAdapter<
                Membership,
                MembershipServiceAdapter,
                StorageAdapter,
                ApiAdapter,
                RuntimeServiceId,
            >
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Clone
                + Debug
                + Send
                + Sync
                + 'static,
            MembershipServiceAdapter: MembershipAdapter,
            ApiAdapter: ApiAdapterTrait<
                    Share = DaShare,
                    BlobId = BlobId,
                    Commitments = DaSharesCommitments,
                    Membership = DaMembershipHandler<Membership>,
                > + Clone
                + Send
                + Sync
                + 'static,
        {
            type Backend = $DaNetworkBackend<Membership>;
            type Settings = ();
            type Share = DaShare;
            type Membership = Membership;
            type Storage = StorageAdapter;
            type MembershipAdapter = MembershipServiceAdapter;
            type ApiAdapter = ApiAdapter;

            async fn new(
                _settings: Self::Settings,
                network_relay: OutboundRelay<
                    <NetworkService<
                        Self::Backend,
                        Self::Membership,
                        Self::MembershipAdapter,
                        Self::Storage,
                        Self::ApiAdapter,
                        RuntimeServiceId,
                    > as ServiceData>::Message,
                >,
            ) -> Self {
                Self {
                    network_relay,
                    _membership: Default::default(),
                }
            }

            async fn share_stream(&self) -> Box<dyn Stream<Item = Self::Share> + Unpin + Send> {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                self.network_relay
                    .send(nomos_da_network_service::DaNetworkMsg::Subscribe {
                        kind: $DaNetworksEventKind::Verifying,
                        sender,
                    })
                    .await
                    .expect("Network backend should be ready");

                let receiver = receiver.await.expect("Blob stream should be received");

                let stream = receiver.filter_map(move |msg| match msg {
                    $DaNetworkEvent::Verifying(verification_event) => match verification_event {
                        VerificationEvent::Share(share) => Some(*share),
                        VerificationEvent::Tx(_) => None,
                    },
                    _ => None,
                });

                Box::new(Box::pin(stream))
            }
        }
    };
}

pub(crate) use adapter_for;
