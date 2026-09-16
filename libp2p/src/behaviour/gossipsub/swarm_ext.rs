use libp2p::{
    PeerId,
    gossipsub::{IdentTopic, MessageId, PublishError, SubscriptionError, TopicHash},
};
use rand::RngCore;

use crate::Swarm;

impl<R: Clone + Send + RngCore + 'static> Swarm<R> {
    /// Subscribes to a topic
    ///
    /// Returns true if the topic is newly subscribed or false if already
    /// subscribed.
    pub fn subscribe(&mut self, topic: &str) -> Result<bool, SubscriptionError> {
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .subscribe(&IdentTopic::new(topic))
    }

    pub fn broadcast<Message>(
        &mut self,
        topic: &str,
        message: Message,
    ) -> Result<MessageId, PublishError>
    where
        Message: Into<Vec<u8>>,
    {
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .publish(IdentTopic::new(topic), message)
    }

    /// Unsubscribes from a topic
    ///
    /// Returns true if previously subscribed
    pub fn unsubscribe(&mut self, topic: &str) -> bool {
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .unsubscribe(&IdentTopic::new(topic))
    }

    pub fn is_subscribed(&mut self, topic: &str) -> bool {
        let topic_hash = topic_hash(topic);

        //TODO: consider O(1) searching by having our own data structure
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .topics()
            .any(|h| h == &topic_hash)
    }

    pub fn blacklist_peer(&mut self, peer_id: PeerId) {
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .blacklist_peer(&peer_id);
    }

    pub fn remove_blacklisted_peer(&mut self, peer_id: PeerId) {
        self.swarm
            .behaviour_mut()
            .inner
            .gossipsub
            .remove_blacklisted_peer(&peer_id);
    }
}

#[must_use]
pub fn topic_hash(topic: &str) -> TopicHash {
    IdentTopic::new(topic).hash()
}
