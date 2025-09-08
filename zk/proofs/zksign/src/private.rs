use std::fmt::Display;

use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::Serialize;

pub struct ZkSignPrivateKeysData([Fr; 32]);

pub struct ZkSignPrivateKeysInputs([Groth16Input; 32]);

#[derive(Serialize)]
#[serde(transparent)]
pub struct ZkSignPrivateKeysInputsJson([Groth16InputDeser; 32]);

impl From<[Fr; 32]> for ZkSignPrivateKeysData {
    fn from(value: [Fr; 32]) -> Self {
        Self(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub struct PrivateKeysTryFromError(usize);

impl Display for PrivateKeysTryFromError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Size should be 32, got {}", self.0)
    }
}
impl TryFrom<&[Fr]> for ZkSignPrivateKeysData {
    type Error = PrivateKeysTryFromError;
    fn try_from(value: &[Fr]) -> Result<Self, Self::Error> {
        let len = value.len();
        if len > 32 {
            return Err(PrivateKeysTryFromError(len));
        }
        let mut buff: [Fr; 32] = [Fr::ZERO; 32];
        buff.copy_from_slice(&value[..len]);
        Ok(Self(buff))
    }
}

impl From<ZkSignPrivateKeysData> for ZkSignPrivateKeysInputs {
    fn from(value: ZkSignPrivateKeysData) -> Self {
        Self(
            value
                .0
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
        )
    }
}

impl From<&ZkSignPrivateKeysInputs> for ZkSignPrivateKeysInputsJson {
    fn from(value: &ZkSignPrivateKeysInputs) -> Self {
        Self(
            value
                .0
                .iter()
                .map(Into::into)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
        )
    }
}
