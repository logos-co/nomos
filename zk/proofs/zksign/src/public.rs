use groth16::{Groth16Input, Groth16InputDeser};
use serde::{Deserialize, Serialize};

pub struct ZkSignVerifierInputs {
    pub public_keys: [Groth16Input; 32],
    pub msg: Groth16Input,
}

#[derive(Deserialize, Serialize)]
pub struct ZkSignVerifierInputsJson {
    public_keys: [Groth16InputDeser; 32],
    msg: Groth16InputDeser,
}

impl TryFrom<ZkSignVerifierInputsJson> for ZkSignVerifierInputs {
    type Error = <Groth16Input as TryFrom<Groth16InputDeser>>::Error;

    fn try_from(value: ZkSignVerifierInputsJson) -> Result<Self, Self::Error> {
        Ok(Self {
            public_keys: value
                .public_keys
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .unwrap_or_else(|_| panic!("Size should be 32")),
            msg: value.msg.try_into()?,
        })
    }
}
