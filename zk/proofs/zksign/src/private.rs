use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::Serialize;

use crate::SecretKey;

pub struct ZkSignPrivateKeysData([Fr; 32]);

pub struct ZkSignPrivateKeysInputs(pub(crate) [Groth16Input; 32]);

#[derive(Serialize)]
#[serde(transparent)]
pub struct ZkSignPrivateKeysInputsJson([Groth16InputDeser; 32]);

impl From<[Fr; 32]> for ZkSignPrivateKeysData {
    fn from(value: [Fr; 32]) -> Self {
        Self(value)
    }
}

impl TryFrom<&[SecretKey]> for ZkSignPrivateKeysData {
    type Error = crate::ZkSignError;

    fn try_from(value: &[SecretKey]) -> Result<Self, Self::Error> {
        let len = value.len();
        if len > 32 {
            return Err(crate::ZkSignError::TooManyKeys(len));
        }
        let mut buff: [Fr; 32] = [Fr::ZERO; 32];
        for (i, sk) in value.iter().enumerate() {
            buff[i] = sk.clone().into_inner();
        }
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
