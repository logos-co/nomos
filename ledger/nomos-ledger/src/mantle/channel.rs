
pub struct ChannelId([u8; 32]);
pub struct MsgId([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channels {
    pub channels: rpds::HashTrieMapSync<ChannelId, ChannelState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelState {
    tip: MsgId,
    // avoid cloning the keys every new message
    keys: Arc<Vec<PublicKey>>,
}

impl Channels {
    pub fn apply_msg<Id>(
        &self,
        channel_id: ChannelId,
        parent: MsgId,
        msg: &MsgId,
        signer: &PublicKey,
    ) -> Result<Self, LedgerError<Id>> {
        let channel = self
            .channels
            .entry(&channel_id)
            .ok_or(LedgerError::ChannelNotFound(channel_id))?;
        if parent != self.tip {
            return Err(LedgerError::InvalidParent {
                channel_id: ChannelId(channel_id.0),
                parent: parent.0,
            });
        }

        if !self.keys.contains(signer) {
            return Err(LedgerError::UnauthorizedSigner {
                channel_id: ChannelId(self.tip.0),
                signer: signer.to_string(),
            });
        }

        Ok(Self {
            tip: *msg,
            keys: self.keys.clone(),
        })
    }

    pub fn set_keys<Id>(& self, keys: Vec<PublicKey>, signer: &PublicKey) -> Result<Self, LedgerError<Id>> {
        // The first key is the admin key
        if self.keys[0] != *signer {
            return Err(LedgerError::UnauthorizedSigner {
                channel_id: ChannelId(self.tip.0),
                signer: signer.to_string(),
            });
        }

        if keys.is_empty() {
            return Err(LedgerError::EmptyKeys {
                channel_id: ChannelId(self.tip.0),
            });
        }

        Ok(Self {
            tip: self.tip,
            keys: Arc::new(keys),
        })
    }

    pub fn new<Id>(tip: MsgId, keys: Vec<PublicKey>) -> Result<Self, LedgerError<Id>> {
        if keys.is_empty() {
            return Err(LedgerError::EmptyKeys {
                channel_id: ChannelId(tip.0),
            });
        }
        Self {
            tip,
            keys: Arc::new(keys),
        }
    }
}