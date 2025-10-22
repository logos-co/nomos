use tokio::sync::oneshot;

/// Messages that the Core service can handle.
pub enum ServiceMessage<CommonServiceMessage> {
    /// Common message type shared across all kinds of Blend services.
    Common(CommonServiceMessage),

    /// Notify that the session transition period has finished.
    ///
    /// This message is mainly used when the core service is being shut down
    /// after the session transition period, because the node is not in the
    /// core role in the current session. In such cases, the core service
    /// may not automatically detect the end of the transition period,
    /// so this message is used to trigger the cleanup logic explicitly.
    ///
    /// When the node remains in the core role for the current session, this
    /// message is unnecessary — the core service continues running and
    /// automatically detects and handles the end of the session transition.
    FinishSessionTransition {
        /// To notify back when the operation is complete
        reply_sender: oneshot::Sender<()>,
    },
}
