use circuits_utils::dev_mode::DevModeProof;
use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::Deserialize;

pub struct ZkSignVerifierInputs {
    pub public_keys: [Groth16Input; 32],
    pub msg: Groth16Input,
}

impl ZkSignVerifierInputs {
    #[must_use]
    pub fn new_from_msg_and_pks(msg: Fr, pks: &[Fr; 32]) -> Self {
        Self {
            msg: msg.into(),
            public_keys: pks
                .iter()
                .map(|pk| (*pk).into())
                .collect::<Vec<_>>()
                .try_into()
                .expect("Size is already check from the function signature"),
        }
    }

    pub(crate) fn from_witness(witness: &crate::ZkSignWitnessInputs) -> Self {
        Self {
            msg: witness.msg,
            public_keys: witness.private_keys.0.map(|input| {
                Groth16Input::from(
                    crate::SecretKey::from(input.into_inner())
                        .to_public_key()
                        .into_inner(),
                )
            }),
        }
    }

    pub fn as_inputs(&self) -> [Fr; 33] {
        let mut buff = [Fr::ZERO; 33];
        buff[..32].copy_from_slice(self.public_keys.map(Groth16Input::into_inner).as_ref());
        buff[32] = self.msg.into_inner();
        buff
    }
}

impl DevModeProof for ZkSignVerifierInputs {
    fn vk(&self) -> &groth16::Groth16PreparedVerificationKey {
        crate::verification_key::ZKSIGN_VK.as_ref()
    }

    fn public_inputs(&self) -> Vec<Groth16Input> {
        std::iter::once(self.msg).chain(self.public_keys).collect()
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
pub struct ZkSignVerifierInputsJson(Vec<Groth16InputDeser>);

#[derive(Debug, thiserror::Error)]
pub enum ZkSignVerifierInputsJsonTryFromError {
    #[error("Error during deserialization: {0:?}")]
    Groth16DeserError(<Groth16Input as TryFrom<Groth16InputDeser>>::Error),
    #[error("Size should be 32")]
    SizeShould32,
    #[error("Empty slice")]
    EmptySlice,
}
impl TryFrom<ZkSignVerifierInputsJson> for ZkSignVerifierInputs {
    type Error = ZkSignVerifierInputsJsonTryFromError;

    fn try_from(mut value: ZkSignVerifierInputsJson) -> Result<Self, Self::Error> {
        let msg = value.0.pop().ok_or(Self::Error::EmptySlice)?;
        Ok(Self {
            public_keys: value
                .0
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()
                .map_err(Self::Error::Groth16DeserError)?
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
            msg: msg.try_into().map_err(Self::Error::Groth16DeserError)?,
        })
    }
}
