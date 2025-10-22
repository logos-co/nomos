use tokio::sync::oneshot;

pub enum ServiceMessage<CommonServiceMessage> {
    Common(CommonServiceMessage),
    FinishSessionTransitionPeriod { reply_sender: oneshot::Sender<()> },
}
