use libp2p::gossipsub::{IdentTopic, MessageId, PublishError, SubscriptionError, TopicHash};

use crate::swarm::{exp_backoff, Swarm, MAX_RETRY};

pub type Topic = String;

#[derive(Debug)]
pub struct BroadcastRequest {
    topic: Topic,
    message: Vec<u8>,
}

#[derive(Debug)]
pub struct InternalBroadcastRequest {
    request: BroadcastRequest,
    retry_count: usize,
}

impl From<BroadcastRequest> for InternalBroadcastRequest {
    fn from(value: BroadcastRequest) -> Self {
        Self {
            request: value,
            retry_count: 0,
        }
    }
}

impl Swarm {
    pub fn broadcast(&mut self, request: BroadcastRequest) -> Result<(), PublishError> {
        self._broadcast(request.into())
    }

    fn _broadcast(&mut self, request: InternalBroadcastRequest) -> Result<(), PublishError> {
        let InternalBroadcastRequest {
            retry_count,
            request: BroadcastRequest { topic, message },
        } = request;

        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(IdentTopic::new(topic), message)
        {
            Ok(id) => {
                tracing::debug!("broadcasted message with id: {id} tp topic: {topic}");
            }
            Err(PublishError::InsufficientPeers) if retry_count < MAX_RETRY => {
                let increased_retry_count = retry_count.saturating_add(1);
                let wait = exp_backoff(increased_retry_count);
                tracing::error!(
                    "failed to broadcast message to topic due to insufficient peers, trying again in {wait:?}"
                );
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    commands_tx
                        .send(Command::PubSub(PubSubCommand::RetryBroadcast {
                            topic,
                            message,
                            retry_count: retry_count + 1,
                        }))
                        .await
                        .unwrap_or_else(|_| tracing::error!("could not schedule retry"));
                });
                Ok(())
            }
            Err(e) => return e,
        }
    }

    /// Subscribes to a topic
    ///
    /// Returns true if the topic is newly subscribed or false if already
    /// subscribed.
    pub fn subscribe(&mut self, topic: &str) -> Result<bool, SubscriptionError> {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&IdentTopic::new(topic))
    }

    /// Unsubscribes from a topic
    ///
    /// Returns true if previously subscribed
    pub fn unsubscribe(&mut self, topic: &str) -> bool {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .unsubscribe(&IdentTopic::new(topic))
    }

    pub fn is_subscribed(&mut self, topic: &str) -> bool {
        let topic_hash = topic_hash(topic);

        //TODO: consider O(1) searching by having our own data structure
        self.swarm
            .behaviour_mut()
            .gossipsub
            .topics()
            .any(|h| h == &topic_hash)
    }
}

#[must_use]
pub fn topic_hash(topic: &str) -> TopicHash {
    IdentTopic::new(topic).hash()
}
