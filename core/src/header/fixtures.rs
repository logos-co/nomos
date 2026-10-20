use lb_binary_codec::canonical::codec_fixtures;
use lb_cryptarchia_engine::Slot;
use lb_groth16::Fr;
// TODO(RFC-480): the header carries the reward_key of the leader reward; the PoL is bound to
// the header by pol_msg.
// use lb_key_management_system_keys::keys::Ed25519PublicKey;
use lb_key_management_system_keys::keys::ZkPublicKey;

use crate::{
    header::{ContentId, Header, HeaderId},
    // TODO(RFC-480): the header carries the reward_key of the leader reward; the PoL is bound to
    // the header by pol_msg.
    // mantle::ops::leader_claim::VoucherCm,
    proofs::leader_proof::Groth16LeaderProof,
};

codec_fixtures!(HeaderId, Self::from([0x11u8; 32]) => "1111111111111111111111111111111111111111111111111111111111111111");

codec_fixtures!(ContentId, Self::from([0x22u8; 32]) => "2222222222222222222222222222222222222222222222222222222222222222");

// TODO(RFC-480): the header carries the reward_key of the leader reward; the
// PoL is bound to the header by pol_msg, and the wire leads with
// `bedrock_version`.
// // Layout: `slot (8B LE) || parent_block (32B) || body_root (32B) ||
// // proof_of_leadership (224B)` — 296 bytes.
// codec_fixtures!(
//     Header,
//     Self::new(
//         HeaderId::from([0x11u8; 32]),
//         ContentId::from([0x22u8; 32]),
//         Slot::from(42u64),
//         Groth16LeaderProof::from_parts(
//             lb_pol::PoLProof::from_bytes(&[0x22u8; _]),
//             Fr::from(0x5555u64),
//             Ed25519PublicKey::from_bytes(&[0x33u8; _]).unwrap(),
//             VoucherCm::from(Fr::from(0x4444u64)),
//         ),
//     ) => "2a00000000000000111111111111111111111111111111111111111111111111111111111111111122222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222555500000000000000000000000000000000000000000000000000000000000033333333333333333333333333333333333333333333333333333333333333334444000000000000000000000000000000000000000000000000000000000000"
// );

// Layout: `bedrock_version (1B) || parent_block (32B) || slot (8B LE) ||
// body_root (32B) || proof_of_leadership (192B)`, 265 bytes.
codec_fixtures!(
    Header,
    Self::new(
        HeaderId::from([0x11u8; 32]),
        ContentId::from([0x22u8; 32]),
        Slot::from(42u64),
        Groth16LeaderProof::from_parts(
            lb_pol::PoLProof::from_bytes(&[0x22u8; _]),
            Fr::from(0x5555u64),
            ZkPublicKey::new(Fr::from(0x4444u64)),
        ),
    ) => "0111111111111111111111111111111111111111111111111111111111111111112a000000000000002222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222222255550000000000000000000000000000000000000000000000000000000000004444000000000000000000000000000000000000000000000000000000000000"
);
