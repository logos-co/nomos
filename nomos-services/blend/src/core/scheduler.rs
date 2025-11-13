use core::ops::{Deref, DerefMut};

use nomos_blend_scheduling::{
    MessageScheduler,
    message_scheduler::{ProcessedMessageScheduler, Settings, session_info::SessionInfo},
};

/// A wrapper around a [`MessageScheduler`] that allows creation with a set of
/// initial messages.
pub struct SchedulerWrapper<SessionClock, Rng, ProcessedMessage, DataMessage> {
    /// The inner message scheduler.
    scheduler: MessageScheduler<SessionClock, Rng, ProcessedMessage, DataMessage>,
}

impl<SessionClock, Rng, ProcessedMessage, DataMessage>
    SchedulerWrapper<SessionClock, Rng, ProcessedMessage, DataMessage>
where
    Rng: rand::Rng + Clone,
{
    pub fn new_with_initial_messages<ProcessedMessages, DataMessages>(
        session_clock: SessionClock,
        initial_session_info: SessionInfo,
        rng: Rng,
        settings: Settings,
        processed_messages: ProcessedMessages,
        data_messages: DataMessages,
    ) -> Self
    where
        ProcessedMessages: Iterator<Item = ProcessedMessage>,
        DataMessages: Iterator<Item = DataMessage>,
    {
        let mut inner_scheduler =
            MessageScheduler::new(session_clock, initial_session_info, rng, settings);
        processed_messages.for_each(|m| inner_scheduler.schedule_processed_message(m));
        data_messages.for_each(|m| inner_scheduler.queue_data_message(m));
        Self {
            scheduler: inner_scheduler,
        }
    }
}

impl<SessionClock, Rng, ProcessedMessage, DataMessage> Deref
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage, DataMessage>
{
    type Target = MessageScheduler<SessionClock, Rng, ProcessedMessage, DataMessage>;

    fn deref(&self) -> &Self::Target {
        &self.scheduler
    }
}

impl<SessionClock, Rng, ProcessedMessage, DataMessage> DerefMut
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage, DataMessage>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.scheduler
    }
}

impl<SessionClock, Rng, ProcessedMessage, DataMessage> ProcessedMessageScheduler<ProcessedMessage>
    for SchedulerWrapper<SessionClock, Rng, ProcessedMessage, DataMessage>
{
    fn schedule_processed_message(&mut self, message: ProcessedMessage) {
        self.scheduler.schedule_processed_message(message);
    }
}
