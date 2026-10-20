use std::fmt::{Debug, Formatter};

use lb_core::mantle::notes::{NoteCm, NoteNf, derive_note_nf};
use lb_key_management_system_keys::keys::{
    ZkKey, errors::KeyError, secured_key::SecureKeyOperator,
};
use lb_log_targets::kms;
use tokio::sync::oneshot;
use tracing::debug;

const LOG_TARGET: &str = kms::operators::ZK_NULLIFIER;

pub struct DeriveNullifier {
    cm: NoteCm,
    result_channel: oneshot::Sender<NoteNf>,
}

impl Debug for DeriveNullifier {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeriveNullifier")
    }
}

impl DeriveNullifier {
    #[must_use]
    pub const fn new(cm: NoteCm, result_channel: oneshot::Sender<NoteNf>) -> Self {
        Self { cm, result_channel }
    }
}

#[async_trait::async_trait]
impl SecureKeyOperator for DeriveNullifier {
    type Key = ZkKey;
    type Error = KeyError;

    async fn execute(self: Box<Self>, key: &Self::Key) -> Result<(), Self::Error> {
        let Self { cm, result_channel } = *self;
        if result_channel
            .send(derive_note_nf(&cm, key.as_fr()))
            .is_err()
        {
            debug!(target: LOG_TARGET, "Failed to send result via channel");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use lb_groth16::Fr;
    use lb_key_management_system_keys::keys::secured_key::SecuredKey as _;

    use super::*;

    #[tokio::test]
    async fn derive_nullifier_hashes_the_commitment_with_the_key() {
        let key = ZkKey::new(Fr::from(7u64));
        let cm = NoteCm(Fr::from(3u64));
        let (sender, receiver) = oneshot::channel();
        key.execute(DeriveNullifier::new(cm, sender)).await.unwrap();
        assert_eq!(
            receiver.await.unwrap(),
            derive_note_nf(&cm, &Fr::from(7u64))
        );
    }
}
