macro_rules! adapter_for {
    ($DaNetworkBackend:ident, $DaNetworkMessage:ident, $DaEventKind:ident, $DaNetworkEvent:ident) => {
        pub struct Libp2pAdapter<Membership, RuntimeServiceId>
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Debug
                + Clone
                + Send
                + Sync
                + 'static,
        {
            network_relay: OutboundRelay<
                <NetworkService<$DaNetworkBackend<Membership>, Membership, RuntimeServiceId> as ServiceData>::Message,
            >,
        }

        #[async_trait::async_trait]
        impl<Membership, RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for Libp2pAdapter<Membership, RuntimeServiceId>
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Debug
                + Clone
                + Send
                + Sync
                + 'static,
        {
            type Backend = $DaNetworkBackend<Membership>;
            type Settings = ();
            type Membership = Membership;

            async fn new(
                network_relay: OutboundRelay<<NetworkService<Self::Backend, Membership, RuntimeServiceId> as ServiceData>::Message>,
            ) -> Self {
                Self { network_relay }
            }

            async fn start_sampling(
                &mut self,
                blob_id: BlobId,
                subnets: &[SubnetworkId],
            ) -> Result<(), DynError> {
                for id in subnets {
                    let subnetwork_id = id;
                    self.network_relay
                        .send(DaNetworkMsg::Process($DaNetworkMessage::RequestSample {
                            blob_id,
                            subnetwork_id: *subnetwork_id,
                        }))
                        .await
                        .expect("RequestSample message should have been sent")
                }
                Ok(())
            }

            async fn listen_to_sampling_messages(
                &self,
            ) -> Result<Pin<Box<dyn Stream<Item = SamplingEvent> + Send>>, DynError> {
                let (stream_sender, stream_receiver) = oneshot::channel();
                self.network_relay
                    .send(DaNetworkMsg::Subscribe {
                        kind: $DaEventKind::Sampling,
                        sender: stream_sender,
                    })
                    .await
                    .map_err(|(error, _)| error)?;
                stream_receiver
                    .await
                    .map(|stream| {
                        tokio_stream::StreamExt::filter_map(stream, |event| match event {
                            $DaNetworkEvent::Sampling(event) => {
                                Some(event)
                            }
                            _ => {
                                unreachable!("Subscribing to sampling events should return a sampling only event stream");
                            }
                        }).boxed()
                    })
                    .map_err(|error| Box::new(error) as DynError)
            }
        }
    }
}

pub(crate) use adapter_for;
