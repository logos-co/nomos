#[cfg(test)]
mod tests {
    use lb_cryptarchia_engine::Slot;

    // TODO(RFC-480): the proof of leadership is bound to the header by pol_msg, blocks carry
    // no signature.
    // use lb_key_management_system_keys::keys::Ed25519Key;
    use crate::{
        block::{Block, BlockTransactions, UncleHeaders, tests::create_proof},
        mantle::transactions::Ops,
    };

    fn make_empty_block() -> Block<Ops> {
        Block::create(
            [0u8; 32].into(),
            Slot::from(1u64),
            UncleHeaders::empty(),
            create_proof(),
            BlockTransactions::empty(),
        )
        .expect("block creation should succeed")
    }

    #[test]
    fn test_json_round_trip() {
        let block = make_empty_block();
        let json = serde_json::to_string(&block).expect("JSON serialization should succeed");
        let restored: Block<Ops> =
            serde_json::from_str(&json).expect("JSON deserialization should succeed");
        assert_eq!(block.header().id(), restored.header().id());
        // TODO(RFC-480): the proof of leadership is bound to the header by
        // pol_msg, blocks carry no signature.
        // assert_eq!(block.signature(), restored.signature());
    }

    // TODO(RFC-480): the proof of leadership is bound to the header by pol_msg,
    // blocks carry no signature.
    // #[test]
    // fn test_json_signature_is_hex() {
    //     let block = make_empty_block();
    //     let json = serde_json::to_string(&block)
    //         .expect("JSON serialization should succeed");
    //     let value: serde_json::Value =
    //         serde_json::from_str(&json).expect("should parse as JSON");
    //     let sig = value["signature"]
    //         .as_str()
    //         .expect("signature should be a string");
    //     assert_eq!(sig.len(), 128, "Ed25519 signature hex should be 128 chars");
    //     assert!(
    //         sig.chars().all(|c| c.is_ascii_hexdigit()),
    //         "signature should be hex"
    //     );
    // }

    #[test]
    fn test_json_proof_is_hex() {
        let block = make_empty_block();
        let json = serde_json::to_string(&block).expect("JSON serialization should succeed");
        let value: serde_json::Value = serde_json::from_str(&json).expect("should parse as JSON");
        let proof = value["header"]["proof_of_leadership"]["proof"]
            .as_str()
            .expect("proof should be a string");
        assert_eq!(proof.len(), 256, "PoLProof hex should be 256 chars");
        assert!(
            proof.chars().all(|c| c.is_ascii_hexdigit()),
            "proof should be hex"
        );
    }

    #[test]
    fn test_bincode_round_trip() {
        let block = make_empty_block();
        let bytes = bincode::serialize(&block).expect("bincode serialization should succeed");
        let restored: Block<Ops> =
            bincode::deserialize(&bytes).expect("bincode deserialization should succeed");
        assert_eq!(block.header().id(), restored.header().id());
        // TODO(RFC-480): the proof of leadership is bound to the header by
        // pol_msg, blocks carry no signature.
        // assert_eq!(block.signature(), restored.signature());
    }

    #[test]
    fn test_bincode_fixed_size_fields_have_no_length_prefix() {
        const PARENT_BLOCK: usize = 32;
        const SLOT: usize = 8;
        const BODY_ROOT: usize = 32;
        const POL_PROOF: usize = 128;
        const ENTROPY_CONTRIBUTION: usize = 32;
        // TODO(RFC-480): the header carries the reward_key of the leader reward; the
        // PoL is bound to the header by pol_msg.
        // const LEADER_KEY: usize = 32;
        // const VOUCHER_CM: usize = 32;
        const REWARD_KEY: usize = 32;
        // TODO(RFC-480): the proof of leadership is bound to the header by pol_msg,
        // blocks carry no signature.
        // const SIGNATURE: usize = 64;
        const TX_COUNT: usize = 8; // u64 Vec length (genuinely variable)
        // bincode uses 8-byte for a length prefix, unlike the wire codec.
        const UNCLE_HEADERS_COUNT: usize = 8;
        const EXPECTED: usize = PARENT_BLOCK
            + SLOT
            + BODY_ROOT
            + UNCLE_HEADERS_COUNT
            + POL_PROOF
            + ENTROPY_CONTRIBUTION
            // TODO(RFC-480): the header carries the reward_key of the leader reward; the PoL is
            // bound to the header by pol_msg.
            // + LEADER_KEY
            // + VOUCHER_CM
            + REWARD_KEY
            // TODO(RFC-480): the proof of leadership is bound to the header by pol_msg, blocks
            // carry no signature.
            // + SIGNATURE
            + TX_COUNT;

        let block = make_empty_block();
        let bytes = bincode::serialize(&block).expect("bincode serialization should succeed");
        assert_eq!(
            bytes.len(),
            EXPECTED,
            "empty block encoding must contain no length prefixes for fixed-size fields"
        );
    }
}
