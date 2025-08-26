use core::ops::{Deref, DerefMut};

use libp2p::StreamProtocol as Libp2pStreamProtocol;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Struct wrapping around a `StreamProtocol` to make it serializable and
/// catch any invalid protocol names already at config reading instead of later
/// on.
pub struct StreamProtocol(
    #[serde(
        serialize_with = "serialize_stream_protocol",
        deserialize_with = "deserialize_stream_protocol"
    )]
    Libp2pStreamProtocol,
);

fn serialize_stream_protocol<S>(
    protocol_name: &Libp2pStreamProtocol,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    protocol_name.as_ref().serialize(serializer)
}

fn deserialize_stream_protocol<'de, D>(deserializer: D) -> Result<Libp2pStreamProtocol, D::Error>
where
    D: Deserializer<'de>,
{
    let protocol_name = String::deserialize(deserializer)?;
    Libp2pStreamProtocol::try_from_owned(protocol_name).map_err(Error::custom)
}

impl StreamProtocol {
    #[must_use]
    pub const fn new(protocol_name: &'static str) -> Self {
        Self(Libp2pStreamProtocol::new(protocol_name))
    }

    #[must_use]
    pub fn into_inner(self) -> Libp2pStreamProtocol {
        self.0
    }
}

impl Deref for StreamProtocol {
    type Target = Libp2pStreamProtocol;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StreamProtocol {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<StreamProtocol> for Libp2pStreamProtocol {
    fn from(value: StreamProtocol) -> Self {
        value.0
    }
}

impl From<Libp2pStreamProtocol> for StreamProtocol {
    fn from(value: Libp2pStreamProtocol) -> Self {
        Self(value)
    }
}
