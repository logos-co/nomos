macro_rules! adapter_for {
    ($DaNetworkBackend:ident, $DaNetworksEventKind:ident, $DaNetworkEvent:ident) => {
        pub struct Libp2pAdapter<M, RuntimeServiceId>
        where
            M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Clone
                + Debug
                + Send
                + Sync
                + 'static,
        {
            network_relay: OutboundRelay<
                <NetworkService<$DaNetworkBackend<M>, RuntimeServiceId> as ServiceData>::Message,
            >,
            _membership: PhantomData<M>,
        }

        #[async_trait::async_trait]
        impl<M, RuntimeServiceId> NetworkAdapter<RuntimeServiceId>
            for Libp2pAdapter<M, RuntimeServiceId>
        where
            M: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Clone
                + Debug
                + Send
                + Sync
                + 'static,
        {
            type Backend = $DaNetworkBackend<M>;
            type Settings = ();
            type Share = DaShare;

            async fn new(
                _settings: Self::Settings,
                network_relay: OutboundRelay<
                    <NetworkService<Self::Backend, RuntimeServiceId> as ServiceData>::Message,
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
