use futures::Stream;
use nomos_blend_message::crypto::proofs::quota::inputs::prove::private::ProofOfLeadershipQuotaInputs;

use crate::message::PolEpochInfo;

pub const fn mock_pol_epoch() -> PolEpochInfo {
    use groth16::Field as _;
    use nomos_core::crypto::ZkHash;

    PolEpochInfo {
        epoch_nonce: ZkHash::ZERO,
        poq_private_inputs: ProofOfLeadershipQuotaInputs {
            aged_path: vec![],
            aged_selector: vec![],
            note_value: 0,
            output_number: 0,
            pol_secret_key: ZkHash::ZERO,
            slot: 0,
            slot_secret: ZkHash::ZERO,
            slot_secret_path: vec![],
            starting_slot: 0,
            transaction_hash: ZkHash::ZERO,
        },
    }
}

pub fn mock_pol_epoch_stream() -> impl Stream<Item = PolEpochInfo> {
    use futures::stream::repeat;

    repeat(mock_pol_epoch())
}
