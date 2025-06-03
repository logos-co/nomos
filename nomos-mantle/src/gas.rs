pub type Gas = u64;

pub trait GasPrice {
    fn gas_price<Constants: GasConstants>(&self) -> Gas;
}

pub trait GasConstants {
    /// Verify the proof of ownership and relative balance.
    const TX_BASE_GAS: Gas;

    /// Check note identifiers existence and if they are spendable. Remove note
    /// identifiers from the set.
    const INPUT_GAS: Gas;

    /// Add note identifiers to the set and store the associated witness.
    const OUTPUT_GAS: Gas;

    /// Verify the inscription signature.
    const INSCRIBE_BASE_GAS: Gas;

    /// Gas per inscription byte.
    const INSCRIBE_BYTE_GAS: Gas;

    /// Additional cost if this inscription is ordered.
    const INSCRIBE_ORDERING_GAS: Gas;

    /// Verify blob availability and signature.
    const BLOB_BASE_GAS: Gas;

    /// Store the message in a blob.
    const BLOB_BYTE_GAS: Gas;

    /// Additional cost if this blob is ordered.
    const BLOB_ORDERING_GAS: Gas;

    /// Verify the administrator signature.
    const SET_CHANNEL_KEYS_BASE_GAS: Gas;

    /// Gas per byte for key.
    const SET_CHANNEL_KEY_BYTE_GAS: Gas;

    /// Verify the proof of ownership.
    const DECLARE_BASE_GAS: Gas;

    /// Cost of declaration scales with number of locators
    const DECLARE_LOCATOR_GAS: Gas;

    /// Verify the proof of ownership.
    const WITHDRAW_BASE_GAS: Gas;

    /// Store the active message.
    const ACTIVE_BASE_GAS: Gas;

    /// Store the message on the ledger and process it.
    const ACTIVE_BYTE_GAS: Gas;

    /// Consume a reward ticket.
    const CLAIM_BASE_GAS: Gas;
}
