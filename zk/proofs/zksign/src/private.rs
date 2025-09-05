use groth16::{Fr, Groth16Input, Groth16InputDeser};
use serde::Serialize;

pub struct ZkSignPrivateKeysData([Fr; 32]);

pub struct ZkSignPrivateKeysInputs {
    private_keys: [Groth16Input; 32],
}

#[derive(Serialize)]
pub struct ZkSignPrivateKeysInputsJson {
    #[serde(flatten)]
    private_keys: [Groth16InputDeser; 32],
}

impl From<[Fr; 32]> for ZkSignPrivateKeysData {
    fn from(value: [Fr; 32]) -> Self {
        Self(value)
    }
}

impl From<ZkSignPrivateKeysData> for ZkSignPrivateKeysInputs {
    fn from(value: ZkSignPrivateKeysData) -> Self {
        Self {
            private_keys: value
                .0
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
        }
    }
}

impl From<&ZkSignPrivateKeysInputs> for ZkSignPrivateKeysInputsJson {
    fn from(value: &ZkSignPrivateKeysInputs) -> Self {
        Self {
            private_keys: value
                .private_keys
                .iter()
                .map(Into::into)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
        }
    }
}
