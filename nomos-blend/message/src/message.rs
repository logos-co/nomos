use serde::{Deserialize, Serialize};

use crate::{
    header::{Header, PrivateHeader, PublicHeader},
    payload::EncapsulatedPayload,
    serde::FixedSizeSerde,
};

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Copy, Clone)]
pub struct Message {
    header: Header,
    public_header: PublicHeader,
    private_header: PrivateHeader,
    payload: EncapsulatedPayload,
}

impl FixedSizeSerde for Message {
    const SIZE: usize =
        Header::SIZE + PublicHeader::SIZE + PrivateHeader::SIZE + EncapsulatedPayload::SIZE;
}

#[cfg(test)]
mod tests {
    use crate::{
        crypto::{ProofOfQuota, PublicKey, Signature},
        header::{
            BlendingHeader, BlendingSubHeader, EncapsulatedBlendingHeader, MAX_ENCAPSULATIONS,
        },
        payload::Payload,
    };

    use super::*;

    #[test]
    fn test_message_serde() {
        let message = Message {
            header: Header { version: 1 },
            public_header: PublicHeader(BlendingSubHeader {
                public_key: PublicKey([2; 32]),
                proof_of_quota: ProofOfQuota([3; 160]),
                signature: Signature([4; 64]),
            }),
            private_header: PrivateHeader(
                [EncapsulatedBlendingHeader([5; BlendingHeader::SIZE]); MAX_ENCAPSULATIONS],
            ),
            payload: EncapsulatedPayload([6; Payload::SIZE]),
        };

        let serialized = message.to_bytes();
        assert_eq!(serialized.len(), Message::SIZE);

        let deserialized = Message::from_bytes(&serialized).unwrap();
        assert_eq!(deserialized, message);

        Message::from_bytes(&serialized[..serialized.len() - 1]).unwrap_err();

        let mut long = serialized.clone();
        long.extend_from_slice(&serialized);
        let deserialized = Message::from_bytes(&long).unwrap();
        assert_eq!(deserialized, message);
    }
}
