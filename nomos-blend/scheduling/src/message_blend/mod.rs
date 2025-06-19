pub mod crypto;
pub mod temporal;

use std::{
    fmt::Debug,
    hash::Hash,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

pub use crypto::CryptographicProcessorSettings;
use futures::{Stream, StreamExt as _};
use nomos_blend_message::{BlendMessage, MessageUnwrapError};
use nomos_utils::math::NonNegativeF64;
use rand::RngCore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
pub use temporal::TemporalSchedulerSettings;
use tokio::sync::{mpsc, mpsc::UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{
    membership::Membership,
    message_blend::{crypto::CryptographicProcessor, temporal::TemporalProcessorExt as _},
    BlendOutgoingMessage,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageBlendSettings<M>
where
    M: BlendMessage,
    M::PrivateKey: Serialize + DeserializeOwned,
{
    pub cryptographic_processor: CryptographicProcessorSettings<M::PrivateKey>,
    pub temporal_processor: TemporalSchedulerSettings,
    // As per the Blend spec
    pub minimum_messages_coefficient: usize,
    /// `alpha`: message number normalization constant
    pub normalization_constant: NonNegativeF64,
}

/// [`MessageBlendStream`] handles the entire blending tiers process
/// - Unwraps incoming messages received from network using
///   [`CryptographicProcessor`]
/// - Pushes unwrapped messages to [`TemporalProcessor`]
pub struct MessageBlendStream<S, NodeId, Rng, M, Scheduler>
where
    M: BlendMessage,
{
    input_stream: S,
    output_stream: Pin<Box<dyn Stream<Item = BlendOutgoingMessage> + Send + Sync + 'static>>,
    temporal_sender: UnboundedSender<BlendOutgoingMessage>,
    cryptographic_processor: CryptographicProcessor<NodeId, Rng, M>,
    _rng: PhantomData<Rng>,
    _scheduler: PhantomData<Scheduler>,
}

impl<S, NodeId, Rng, M, Scheduler> MessageBlendStream<S, NodeId, Rng, M, Scheduler>
where
    S: Stream<Item = Vec<u8>>,
    NodeId: Hash + Eq,
    Rng: RngCore + Unpin + Send + 'static,
    M: BlendMessage,
    M::PrivateKey: Serialize + DeserializeOwned,
    M::PublicKey: Clone + PartialEq,
    M::Error: Debug,
    Scheduler: Stream<Item = ()> + Unpin + Send + Sync + 'static,
{
    pub fn new(
        input_stream: S,
        settings: MessageBlendSettings<M>,
        membership: Membership<NodeId, M>,
        scheduler: Scheduler,
        cryptographic_processor_rng: Rng,
    ) -> Self {
        let cryptographic_processor = CryptographicProcessor::new(
            settings.cryptographic_processor,
            membership,
            cryptographic_processor_rng,
        );
        let (temporal_sender, temporal_receiver) = mpsc::unbounded_channel();
        let output_stream =
            Box::pin(UnboundedReceiverStream::new(temporal_receiver).temporal_stream(scheduler));
        Self {
            input_stream,
            output_stream,
            temporal_sender,
            cryptographic_processor,
            _rng: PhantomData,
            _scheduler: PhantomData,
        }
    }

    fn process_incoming_message(self: &Pin<&mut Self>, message: &[u8]) {
        match self.cryptographic_processor.unwrap_message(message) {
            Ok((unwrapped_message, fully_unwrapped)) => {
                let message = if fully_unwrapped {
                    BlendOutgoingMessage::FullyUnwrapped(unwrapped_message)
                } else {
                    BlendOutgoingMessage::Outbound(unwrapped_message)
                };
                if let Err(e) = self.temporal_sender.send(message) {
                    tracing::error!("Failed to send message to the outbound channel: {e:?}");
                }
            }
            Err(e @ MessageUnwrapError::NotAllowed) => {
                tracing::debug!("{e}");
            }
            Err(e) => {
                tracing::error!("Failed to unwrap message: {e}");
            }
        }
    }
}

impl<S, NodeId, Rng, M, Scheduler> Stream for MessageBlendStream<S, NodeId, Rng, M, Scheduler>
where
    S: Stream<Item = Vec<u8>> + Unpin,
    NodeId: Hash + Eq + Unpin,
    Rng: RngCore + Unpin + Send + 'static,
    M: BlendMessage + Unpin,
    M::PrivateKey: Serialize + DeserializeOwned + Unpin,
    M::PublicKey: Clone + PartialEq + Unpin,
    M::Error: Debug,
    Scheduler: Stream<Item = ()> + Unpin + Send + Sync + 'static,
{
    type Item = BlendOutgoingMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(message)) = self.input_stream.poll_next_unpin(cx) {
            self.process_incoming_message(&message);
        }
        self.output_stream.poll_next_unpin(cx)
    }
}

pub trait MessageBlendExt<NodeId, Rng, M, Scheduler>: Stream<Item = Vec<u8>>
where
    NodeId: Hash + Eq,
    Rng: RngCore + Send + Unpin + 'static,
    M: BlendMessage,
    M::PrivateKey: Serialize + DeserializeOwned,
    M::PublicKey: Clone + PartialEq,
    M::Error: Debug,
    Scheduler: Stream<Item = ()> + Unpin + Send + Sync + 'static,
{
    fn blend(
        self,
        message_blend_settings: MessageBlendSettings<M>,
        membership: Membership<NodeId, M>,
        scheduler: Scheduler,
        cryptographic_processor_rng: Rng,
    ) -> MessageBlendStream<Self, NodeId, Rng, M, Scheduler>
    where
        Self: Sized + Unpin,
    {
        MessageBlendStream::new(
            self,
            message_blend_settings,
            membership,
            scheduler,
            cryptographic_processor_rng,
        )
    }
}

impl<T, NodeId, Rng, M, S> MessageBlendExt<NodeId, Rng, M, S> for T
where
    T: Stream<Item = Vec<u8>>,
    NodeId: Hash + Eq,
    Rng: RngCore + Unpin + Send + 'static,
    M: BlendMessage,
    M::PrivateKey: Clone + Serialize + DeserializeOwned + PartialEq,
    M::PublicKey: Clone + Serialize + DeserializeOwned + PartialEq,
    M::Error: Debug,
    S: Stream<Item = ()> + Unpin + Send + Sync + 'static,
{
}
