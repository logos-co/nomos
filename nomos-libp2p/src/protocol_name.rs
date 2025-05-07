use serde::{Deserialize, Serialize};

pub const MAINNET_KAD_PROTOCOL_NAME: &str = "/nomos/kad/1.0.0";
pub const MAINNET_IDENTIFY_PROTOCOL_NAME: &str = "/nomos/identify/1.0.0";

pub const TESTNET_KAD_PROTOCOL_NAME: &str = "/testnet/nomos/kad/1.0.0";
pub const TESTNET_IDENTIFY_PROTOCOL_NAME: &str = "/testnet/nomos/identify/1.0.0";

pub const UNITTEST_KAD_PROTOCOL_NAME: &str = "/unittest/nomos/kad/1.0.0";
pub const UNITTEST_IDENTIFY_PROTOCOL_NAME: &str = "/unittest/nomos/identify/1.0.0";

pub const INTEGRATION_KAD_PROTOCOL_NAME: &str = "/integration/nomos/kad/1.0.0";
pub const INTEGRATION_IDENTIFY_PROTOCOL_NAME: &str = "/integration/nomos/identify/1.0.0";

/// Network environment type
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolName {
    Mainnet,
    Testnet,
    #[default]
    Unittest,
    Integration,
}

impl ProtocolName {
    #[must_use]
    pub const fn kad_protocol_name(self) -> &'static str {
        match self {
            Self::Mainnet => MAINNET_KAD_PROTOCOL_NAME,
            Self::Testnet => TESTNET_KAD_PROTOCOL_NAME,
            Self::Unittest => UNITTEST_KAD_PROTOCOL_NAME,
            Self::Integration => INTEGRATION_KAD_PROTOCOL_NAME,
        }
    }

    #[must_use]
    pub const fn identify_protocol_name(self) -> &'static str {
        match self {
            Self::Mainnet => MAINNET_IDENTIFY_PROTOCOL_NAME,
            Self::Testnet => TESTNET_IDENTIFY_PROTOCOL_NAME,
            Self::Unittest => UNITTEST_IDENTIFY_PROTOCOL_NAME,
            Self::Integration => INTEGRATION_IDENTIFY_PROTOCOL_NAME,
        }
    }
}
