use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::{Deserialize, Serialize};

pub const CORE_MERKLE_TREE_HEIGHT: usize = 20;

#[derive(Clone)]
pub struct PoQBlendInputs {
    core_sk: Groth16Input,
    core_path: [(Groth16Input, Groth16Input); CORE_MERKLE_TREE_HEIGHT],
}

pub struct PoQBlendInputsData {
    pub core_sk: Fr,
    pub core_path: [(Fr, bool); CORE_MERKLE_TREE_HEIGHT],
}

#[derive(Deserialize, Serialize)]
pub struct PoQBlendInputsJson {
    core_sk: Groth16InputDeser,
    core_path: [(Groth16InputDeser, Groth16InputDeser); CORE_MERKLE_TREE_HEIGHT],
}

impl From<PoQBlendInputs> for PoQBlendInputsJson {
    fn from(PoQBlendInputs { core_sk, core_path }: PoQBlendInputs) -> Self {
        Self {
            core_sk: (&core_sk).into(),
            core_path: core_path.map(|(value, selector)| ((&value).into(), (&selector).into())),
        }
    }
}

impl From<PoQBlendInputsData> for PoQBlendInputs {
    fn from(PoQBlendInputsData { core_sk, core_path }: PoQBlendInputsData) -> Self {
        Self {
            core_sk: core_sk.into(),
            core_path: core_path.map(|(value, selector)| {
                (
                    value.into(),
                    Groth16Input::new(if selector { Fr::ONE } else { Fr::ZERO }),
                )
            }),
        }
    }
}
