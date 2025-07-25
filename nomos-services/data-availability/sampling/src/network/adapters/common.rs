macro_rules! adapter_for {
    ($DaNetworkBackend:ident, $DaNetworkMessage:ident, $DaEventKind:ident, $DaNetworkEvent:ident) => {
        pub struct Libp2pAdapter<Membership,MembershipServiceAdapter, StorageAdapter, ApiAdapter,RuntimeServiceId>
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Debug
                + Clone
                + Send
                + Sync
                + 'static,
            MembershipServiceAdapter: MembershipAdapter,
            ApiAdapter: ApiAdapterTrait,

        {
            network_relay: OutboundRelay<
                <NetworkService<$DaNetworkBackend<Membership>, Membership, MembershipServiceAdapter, StorageAdapter,ApiAdapter, RuntimeServiceId> as ServiceData>::Message,
            >,
        }

        #[async_trait::async_trait]
        impl<Membership, MembershipServiceAdapter, StorageAdapter, ApiAdapter,RuntimeServiceId> NetworkAdapter<RuntimeServiceId> for Libp2pAdapter<Membership, MembershipServiceAdapter, StorageAdapter,ApiAdapter, RuntimeServiceId>
        where
            Membership: MembershipHandler<NetworkId = SubnetworkId, Id = PeerId>
                + Debug
                + Clone
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
            type Membership = Membership;
            type Storage = StorageAdapter;
            type MembershipAdapter = MembershipServiceAdapter;
            type ApiAdapter = ApiAdapter;

            async fn new(
                network_relay: OutboundRelay<<NetworkService<Self::Backend, Self::Membership, Self::MembershipAdapter, Self::Storage, Self::ApiAdapter, RuntimeServiceId> as ServiceData>::Message>,
            ) -> Self {
                Self { network_relay }
            }

            async fn start_sampling(
                &mut self,
                blob_id: BlobId,
            ) -> Result<(), DynError> {
                self.network_relay
                    .send(DaNetworkMsg::Process($DaNetworkMessage::RequestSample {
                        blob_id,
                    }))
                    .await
                    .expect("RequestSample message should have been sent");
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

            async fn get_commitments(
                &self,
                blob_id: BlobId,
            ) -> Result<Option<DaSharesCommitments>, DynError> {
                let (sender, reply_receiver) = oneshot::channel();
                self.network_relay
                    .send(DaNetworkMsg::GetCommitments {
                        blob_id,
                        sender,
                    })
                    .await
                    .expect("RequestCommitments message should have been sent");
                reply_receiver.await.map_err(DynError::from)
            }
        }
    }
}

pub(crate) use adapter_for;
