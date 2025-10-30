use rand::{Rng as _, RngCore};

#[derive(Debug)]
pub struct MessageBatch<ProcessedMessage> {
    processed_messages: Vec<ProcessedMessage>,
    cover_messages_count: usize,
}

impl<ProcessedMessage> MessageBatch<ProcessedMessage> {
    pub(super) fn new(
        processed_messages: Vec<ProcessedMessage>,
        cover_messages_count: usize,
    ) -> Self {
        Self {
            cover_messages_count,
            processed_messages,
        }
    }

    pub fn into_components(self) -> (Vec<ProcessedMessage>, usize) {
        (self.processed_messages, self.cover_messages_count)
    }
}

pub struct MessageQueue<Rng, ProcessedMessage> {
    processed_message_queue: Vec<ProcessedMessage>,
    cover_messages_count: usize,
    rng: Rng,
}

impl<Rng, ProcessedMessage> MessageQueue<Rng, ProcessedMessage> {
    pub fn new(rng: Rng) -> Self {
        Self {
            cover_messages_count: 0,
            processed_message_queue: Vec::new(),
            rng,
        }
    }

    pub fn add_processed_message(&mut self, message: ProcessedMessage) {
        self.processed_message_queue.push(message);
    }

    pub fn add_cover_message(&mut self) {
        self.cover_messages_count = self
            .cover_messages_count
            .checked_add(1)
            .expect("Overflow when computing new cover message amount.");
    }
}

impl<Rng, ProcessedMessage> MessageQueue<Rng, ProcessedMessage>
where
    Rng: RngCore,
{
    pub fn take_next_random_n_elements(
        &mut self,
        n: usize,
    ) -> Option<MessageBatch<ProcessedMessage>> {
        let processed_messages = if n >= self.processed_message_queue.len() {
            self.processed_message_queue
                .drain(0..self.processed_message_queue.len())
                .collect()
        } else {
            let mut processed_messages = Vec::with_capacity(n);

            (0..n).for_each(|_| {
                let random_index = self.rng.gen_range(0..processed_messages.len());
                // We don't care if we change the order of the elements (as is the case with
                // `swap_remove`) since we take them at random anyway.
                processed_messages.push(self.processed_message_queue.swap_remove(random_index));
            });
            processed_messages
        };

        // How many cover messages can we return?
        let missing_messages_to_release = n
            .checked_sub(processed_messages.len())
            .expect("We are guaranteed to not have taken more than `n` elements from the queues.");

        // How many of the missing messages can we fill with cover messages?
        let cover_messages_to_release = self.cover_messages_count.min(missing_messages_to_release);

        self.cover_messages_count = self
            .cover_messages_count
            .checked_sub(cover_messages_to_release)
            .expect("We know we only take the amount of cover messages available.");

        if processed_messages.is_empty() && cover_messages_to_release == 0 {
            None
        } else {
            Some(MessageBatch {
                processed_messages,
                cover_messages_count: cover_messages_to_release,
            })
        }
    }
}
