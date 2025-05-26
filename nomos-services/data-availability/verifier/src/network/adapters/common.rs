macro_rules! adapter_for {
    ($DaNetworkBackend:ident,$MembershipService:ident, $DaNetworksEventKind:ident, $DaNetworkEvent:ident) => {
        pub struct Libp2pAdapter<Membership, MembershipService, RuntimeServiceId>
        where
        Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Clone
                + Debug
                + Send
                + Sync
                + 'static,
        MembershipService: $MembershipService + Send + Sync + 'static,

        {
            network_relay: OutboundRelay<
                <NetworkService<$DaNetworkBackend<Membership>, MembershipService, RuntimeServiceId> as ServiceData>::Message,
            >,
            _membership: PhantomData<Membership>,
        }

        #[async_trait::async_trait]
        impl<Membership, MembershipService, RuntimeServiceId> NetworkAdapter<RuntimeServiceId>
            for Libp2pAdapter<Membership, MembershipService,  RuntimeServiceId>
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                    + Clone
                    + Debug
                     + Send
                     + Sync
                     + 'static,
            MembershipService: $MembershipService + Send + Sync + 'static,
        {
            type Backend = $DaNetworkBackend<Membership>;
            type Settings = ();
            type Share = DaShare;
            type Membership = MembershipService;

            async fn new(
                _settings: Self::Settings,
                network_relay: OutboundRelay<
                    <NetworkService<Self::Backend,Self::Membership, RuntimeServiceId> as ServiceData>::Message,
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
                    $DaNetworkEvent::Verifying(blob) => Some(*blob),
                    _ => None,
                });

                Box::new(Box::pin(stream))
            }
        }
    };
}

pub(crate) use adapter_for;
